use std::future::{Future, poll_fn};
use std::pin::pin;
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};

use serde_json::Value as Json;
use shed_script::vm::{Run, Step};
use shed_script::{Call, Host, HostError, ScriptError};

pub type Answer = (u64, Result<Json, HostError>);

#[derive(Default)]
pub struct State {
    pub now: u64,
    pub calls: Vec<(String, Json)>,
    pub open: Vec<u64>,
    pub most_open: usize,
}

#[derive(Default)]
pub struct Fixture {
    pub answers: Vec<Answer>,
    pub state: Mutex<State>,
}

impl Host for Fixture {
    fn call(&self, method: &str, params: Json) -> Call<'_> {
        let mut state = self.state.lock().unwrap();
        let (at, answer) = match self.answers.len() {
            0 => (
                state.now,
                Err(HostError {
                    name: "ShedError".into(),
                    code: "NO_SUCH_METHOD".into(),
                    message: format!("{method}: no such method"),
                }),
            ),
            n => self.answers[state.calls.len() % n].clone(),
        };
        state.calls.push((method.into(), params));
        state.open.push(at);
        state.most_open = state.most_open.max(state.open.len());
        drop(state);
        let mut answer = Some(answer);
        Box::pin(poll_fn(move |_| {
            let mut state = self.state.lock().unwrap();
            if state.now < at {
                return Poll::Pending;
            }
            let open = state.open.iter().position(|o| *o == at).unwrap();
            state.open.remove(open);
            Poll::Ready(answer.take().unwrap())
        }))
    }
}

pub fn drive(run: &mut Run<'_, Fixture>, host: &Fixture) -> Result<Json, ScriptError> {
    loop {
        if let Step::Done(outcome) = run.step() {
            return outcome;
        }
        let mut settle = pin!(run.settle());
        let mut cx = Context::from_waker(Waker::noop());
        while settle.as_mut().poll(&mut cx).is_pending() {
            let mut state = host.state.lock().unwrap();
            state.now = *state.open.iter().min().expect("waiting with nothing open");
        }
        assert!(
            host.state.lock().unwrap().most_open <= 32,
            "over 32 calls in flight"
        );
    }
}
