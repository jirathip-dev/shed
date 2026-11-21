use std::collections::VecDeque;
use std::mem;

use serde_json::Value as Json;

use crate::ast::Pos;
use crate::builtins::{self, arg};
use crate::value::{Cell, Frame, Handle, SLOT, Settled, Task, Value, Waiter};
use crate::vm::{Done, Flow, Limit, Run, full, type_error};
use crate::{Host, HostError, ScriptError};

pub(crate) struct Asked {
    pub(crate) promise: Handle,
    pub(crate) method: Box<str>,
    pub(crate) pos: Pos,
}

impl<H: Host> Run<'_, H> {
    pub(crate) fn waiting(&self) -> bool {
        self.done.is_none()
            && self.frames.is_empty()
            && self.ready.is_empty()
            && self.answered.is_empty()
    }

    pub(crate) fn resume(&mut self) -> bool {
        if let Some((asked, answer)) = self.answered.pop_front() {
            self.answer(asked, answer);
            return true;
        }
        if let Some((task, outcome)) = self.ready.pop_front() {
            self.switch(task, outcome);
            return true;
        }
        if self.queued.is_empty() && self.flight.is_empty() {
            self.finish();
        }
        false
    }

    fn finish(&mut self) {
        let pos = self.result_pos;
        let error = |name: &str, message| ScriptError {
            name: name.into(),
            message,
            line: pos.line,
            column: pos.column,
            code: None,
            method: None,
        };
        self.done = Some(match self.heap.get(self.main) {
            Some(Cell::Promise {
                state: Settled::Fulfilled(v),
                ..
            }) => self
                .json(*v, "result", 0)
                .map_err(|message| error("TypeError", message)),
            _ => Err(error(
                "Error",
                "the script waits on a promise that nothing settles".into(),
            )),
        });
    }

    pub(crate) fn fail(&mut self, thrown: Value, pos: Pos) {
        if self.done.is_none() {
            self.done = Some(Err(self.uncaught(thrown, pos)));
            self.drop_calls();
        }
    }

    pub(crate) fn drop_calls(&mut self) -> Vec<String> {
        let flight = self.flight.drain(..).map(|(a, _)| a.method);
        let queued = self.queued.drain(..).map(|(a, _)| a.method);
        let names = flight.chain(queued).map(String::from).collect();
        self.answered.clear();
        names
    }

    pub(crate) fn main_pos(&self) -> Pos {
        match self.main_task.and_then(|t| self.heap.get(t)) {
            Some(Cell::Task(task)) if self.promise != self.main || self.frames.is_empty() => {
                self.pos_in(&task.frames)
            }
            _ => self.pos(),
        }
    }

    pub(crate) fn promise_cell(&mut self, left: usize) -> Done<Handle> {
        self.heap
            .alloc(Cell::Promise {
                state: Settled::Pending,
                waiters: Vec::new(),
                left,
            })
            .map_err(full)
    }

    pub(crate) fn end_task(&mut self, outcome: Settled) {
        if self.promise == self.main {
            self.result_pos = self.pos();
            if let Settled::Rejected(thrown) = outcome {
                return self.fail(thrown, self.result_pos);
            }
        }
        self.frames.clear();
        self.stack.clear();
        self.handlers.clear();
        self.wake(Waiter::Adopt(self.promise), outcome);
    }

    fn park(&mut self, keep: usize) -> Done<Handle> {
        let task = Task {
            frames: self.frames.clone(),
            stack: self.stack.get(..keep).unwrap_or_default().to_vec(),
            handlers: self.handlers.clone(),
            promise: Some(self.promise),
        };
        let task = self.heap.alloc(Cell::Task(Box::new(task))).map_err(full)?;
        if self.promise == self.main {
            self.main_task = Some(task);
        }
        self.frames.clear();
        self.stack.clear();
        self.handlers.clear();
        Ok(task)
    }

    fn switch(&mut self, task: Handle, outcome: Settled) {
        let Some(Cell::Task(parked)) = self.heap.cell_mut(task) else {
            return;
        };
        let parked = mem::take(&mut **parked);
        self.promise = parked.promise.unwrap_or(self.main);
        (self.frames, self.stack, self.handlers) = (parked.frames, parked.stack, parked.handlers);
        match outcome {
            Settled::Fulfilled(v) => self.stack.push(v),
            Settled::Rejected(thrown) => self.flow(Flow::Throw(thrown)),
            Settled::Pending => {}
        }
    }

    pub(crate) fn await_value(&mut self, v: Value) -> Done {
        let state = match self.cell(v) {
            Some(Cell::Promise { state, .. }) => *state,
            _ => Settled::Fulfilled(v),
        };
        let task = self.park(self.stack.len().saturating_sub(1))?;
        match (state, v) {
            (Settled::Pending, Value::Ref(p)) => self.wait_on(p, Waiter::Task(task)),
            (outcome, _) => self.ready.push_back((task, outcome)),
        }
        Ok(())
    }

    pub(crate) fn spawn(
        &mut self,
        function: usize,
        scope: Option<Handle>,
        at: usize,
        args: &[Value],
    ) -> Done {
        let held = self.stack.len() + self.frames.len() + self.handlers.len() + args.len();
        self.room(held.saturating_mul(SLOT).saturating_add(96))?;
        let promise = self.promise_cell(0)?;
        self.enter(function, scope, at, args)?;
        let frame = self.frames.pop();
        let stack = self.stack.split_off(at.min(self.stack.len()));
        let caller = self.park(at)?;
        self.ready
            .push_front((caller, Settled::Fulfilled(Value::Ref(promise))));
        self.frames.extend(frame.map(|f| Frame {
            base: f.base.saturating_sub(at),
            ..f
        }));
        self.stack = stack;
        self.promise = promise;
        Ok(())
    }

    fn wait_on(&mut self, promise: Handle, waiter: Waiter) {
        if let Some(Cell::Promise { waiters, .. }) = self.heap.cell_mut(promise) {
            waiters.push(waiter);
        }
    }

    pub(crate) fn wake(&mut self, waiter: Waiter, outcome: Settled) {
        let mut work = VecDeque::from([(waiter, outcome)]);
        while let Some((waiter, mut outcome)) = work.pop_front() {
            let p = match waiter {
                Waiter::Task(task) => {
                    self.ready.push_back((task, outcome));
                    continue;
                }
                Waiter::Adopt(p) | Waiter::Race(p) => p,
                Waiter::All(p, items, at) => {
                    if let Settled::Fulfilled(v) = outcome {
                        if let Some(Cell::Array(xs)) = self.heap.cell_mut(items)
                            && let Some(slot) = xs.get_mut(at)
                        {
                            *slot = v;
                        }
                        let Some(Cell::Promise { left, .. }) = self.heap.cell_mut(p) else {
                            continue;
                        };
                        *left = left.saturating_sub(1);
                        if *left > 0 {
                            continue;
                        }
                        outcome = Settled::Fulfilled(Value::Ref(items));
                    }
                    p
                }
            };
            if let Settled::Fulfilled(Value::Ref(q)) = outcome
                && q != p
                && let Some(Cell::Promise { state, waiters, .. }) = self.heap.cell_mut(q)
            {
                match *state {
                    Settled::Pending => waiters.push(Waiter::Adopt(p)),
                    settled => work.push_back((Waiter::Adopt(p), settled)),
                }
                continue;
            }
            let Some(Cell::Promise { state, waiters, .. }) = self.heap.cell_mut(p) else {
                continue;
            };
            if *state != Settled::Pending {
                continue;
            }
            *state = outcome;
            let waiters = mem::take(waiters);
            if p == self.main
                && let Settled::Rejected(thrown) = outcome
            {
                self.fail(thrown, self.pos());
            }
            work.extend(waiters.into_iter().map(|w| (w, outcome)));
        }
    }

    pub(crate) fn host_path(&self, v: Value) -> Option<String> {
        match (v, self.cell(v)) {
            (Value::Native(b), _) if builtins::names(b) == ("", "shed") => Some(String::new()),
            (_, Some(Cell::HostPath(path))) => Some(path.to_string()),
            _ => None,
        }
    }

    pub(crate) fn host_call(&mut self, method: &str, at: usize, args: &[Value]) -> Done {
        let params = match arg(args, 0) {
            Value::Undefined => Json::Object(serde_json::Map::new()),
            v => self
                .json(v, "params", 0)
                .map_err(|m| Flow::Error("TypeError", m))?,
        };
        let promise = self.promise_cell(0)?;
        if self.calls >= self.limits.calls {
            return Err(Flow::Limit(Limit::Calls));
        }
        self.calls += 1;
        let pos = self.pos();
        self.queued.push_back((
            Asked {
                promise,
                method: method.into(),
                pos,
            },
            params,
        ));
        self.replace(self.stack.len().saturating_sub(at), Value::Ref(promise));
        Ok(())
    }

    fn answer(&mut self, asked: Asked, answer: Result<Json, HostError>) {
        let outcome = match &answer {
            Ok(json) => self.json_value(json).map(Settled::Fulfilled),
            Err(error) => self.host_error(&asked, error).map(Settled::Rejected),
        };
        match outcome {
            Ok(outcome) => {
                self.retries = 0;
                if let Settled::Rejected(thrown) = outcome
                    && self.unheeded(asked.promise)
                {
                    self.fail(thrown, asked.pos);
                }
                self.wake(Waiter::Adopt(asked.promise), outcome);
            }
            Err(Flow::Limit(limit)) => self.stop(limit),
            Err(_) => {
                self.answered.push_front((asked, answer));
                self.retries += 1;
                if self.retries > 2 {
                    self.stop(Limit::Memory);
                } else if !self.heap.collecting() {
                    let roots = self.roots();
                    self.heap.collect(&roots, 1);
                }
            }
        }
    }

    fn unheeded(&self, promise: Handle) -> bool {
        let returned = matches!(
            self.heap.get(self.main),
            Some(Cell::Promise {
                state: Settled::Fulfilled(_),
                ..
            })
        );
        returned
            && matches!(self.heap.get(promise), Some(Cell::Promise { waiters, .. }) if waiters.is_empty())
    }

    fn host_error(&mut self, asked: &Asked, error: &HostError) -> Done<Value> {
        let name = self.string(error.name.as_str())?;
        let message = self.string(error.message.as_str())?;
        let code = self.string(error.code.as_str())?;
        let method = self.string(&*asked.method)?;
        self.alloc(Cell::Object(vec![
            ("name".into(), name),
            ("message".into(), message),
            ("line".into(), Value::Number(asked.pos.line.into())),
            ("column".into(), Value::Number(asked.pos.column.into())),
            ("code".into(), code),
            ("method".into(), method),
        ]))
    }

    pub(crate) fn promise_static(&mut self, name: &str, args: &[Value]) -> Done<Value> {
        let v = arg(args, 0);
        let state = |run: &Self, v: Value| match run.cell(v) {
            Some(Cell::Promise { state, .. }) => Some(*state),
            _ => None,
        };
        match name {
            "resolve" if state(self, v).is_some() => return Ok(v),
            "resolve" | "reject" => {
                let p = self.promise_cell(0)?;
                let outcome = if name == "reject" {
                    Settled::Rejected(v)
                } else {
                    Settled::Fulfilled(v)
                };
                self.wake(Waiter::Adopt(p), outcome);
                return Ok(Value::Ref(p));
            }
            _ => {}
        }
        let Some(Cell::Array(items)) = self.cell(v) else {
            return type_error(format!(
                "Promise.{name} takes an array of promises, not {}",
                self.a(v)
            ));
        };
        let items = items.clone();
        let p = self.promise_cell(items.len())?;
        let array = match name {
            "all" => Some(
                self.heap
                    .alloc(Cell::Array(vec![Value::Undefined; items.len()]))
                    .map_err(full)?,
            ),
            _ => None,
        };
        if let (Some(array), true) = (array, items.is_empty()) {
            self.wake(Waiter::Adopt(p), Settled::Fulfilled(Value::Ref(array)));
        }
        for (at, item) in items.into_iter().enumerate() {
            let waiter = array.map_or(Waiter::Race(p), |a| Waiter::All(p, a, at));
            match (item, state(self, item)) {
                (Value::Ref(q), Some(Settled::Pending)) => self.wait_on(q, waiter),
                (_, Some(settled)) => self.wake(waiter, settled),
                (_, None) => self.wake(waiter, Settled::Fulfilled(item)),
            }
        }
        Ok(Value::Ref(p))
    }
}
