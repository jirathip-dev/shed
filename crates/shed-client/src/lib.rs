//! The one client for shed's socket: blocking, with no async runtime. [`Conn`] is one connection past
//! the handshake, [`Shed`] a server with a pool, [`Calls`] the typed calls over any transport, and
//! [`Stream`] the streamed methods. It reads no clock and no environment and starts no process.
#![forbid(unsafe_code)]

pub mod conn;
#[cfg(feature = "fake")]
pub mod fake;
pub mod shed;
pub mod stream;

use std::collections::BTreeMap;

use serde::Serialize;
use serde::de::{DeserializeOwned, IgnoredAny};
use serde_json::{Map, Value};
pub use shed_proto as proto;
use shed_proto::{
    Account, AccountAdd, AccountName, AccountsParams, AcpSession, AgentInfo, EmitParams, ErrorCode,
    ExecParams, ExecResult, FileParams, Handover, HandoverCheckParams, HandoverParams, Input,
    KeysParams, Link, Login, LsParams, MachineStats, MatchParams, Matched, Method, ModelParams,
    ReadParams, ReadResult, Reported, ResizeParams, RunParams, SendParams, StateParams,
    StatsParams, Target, TerminalInfo, WaitParams, WaitResult, frame,
};

pub use conn::{Conn, Error, notification, once};
pub use shed::Shed;
pub use stream::{Stream, Streamed};

/// The oldest server protocol this client works against; what `hello` carries as `oldest`.
pub const MIN_PROTOCOL: u32 = 3;

fn typed<C, P, R>(calls: &C, method: Method, params: &P) -> Result<R, Error>
where
    C: Calls + ?Sized,
    P: Serialize,
    R: DeserializeOwned,
{
    conn::decode(calls.call_raw(method, conn::encode(params)?)?)
}

fn unit<C: Calls + ?Sized>(
    calls: &C,
    method: Method,
    params: &impl Serialize,
) -> Result<(), Error> {
    typed::<_, _, IgnoredAny>(calls, method, params).map(drop)
}

fn target(name: &str) -> Target {
    let name = name.to_owned();
    Target { name }
}

fn account(provider: &str, name: &str) -> AccountName {
    let (provider, name) = (provider.to_owned(), name.to_owned());
    AccountName { provider, name }
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

/// The typed calls, written once over [`Calls::call_raw`]; every error is one [`Error`].
#[expect(
    clippy::missing_errors_doc,
    reason = "every call fails as `call_raw` does, with the one `Error`"
)]
pub trait Calls {
    /// Calls `method` with `params` and gives back its raw result.
    fn call_raw(&self, method: Method, params: Value) -> Result<Value, Error>;

    /// Starts a terminal.
    fn run(&self, params: &RunParams) -> Result<TerminalInfo, Error> {
        typed(self, Method::Run, params)
    }

    /// The terminals, by name.
    fn ls(&self, params: &LsParams) -> Result<Vec<TerminalInfo>, Error> {
        typed(self, Method::Ls, params)
    }

    /// One terminal.
    fn get(&self, name: &str) -> Result<TerminalInfo, Error> {
        typed(self, Method::Get, &target(name))
    }

    /// A terminal's screen, scrollback or transcript.
    fn read(&self, params: &ReadParams) -> Result<ReadResult, Error> {
        typed(self, Method::Read, params)
    }

    /// A terminal's visible screen as text.
    fn screen(&self, name: &str) -> Result<String, Error> {
        let name = name.to_owned();
        let params = ReadParams {
            name,
            ..ReadParams::default()
        };
        Ok(self.read(&params)?.text)
    }

    /// Pastes text into a terminal, then presses enter.
    fn send(&self, params: &SendParams) -> Result<(), Error> {
        unit(self, Method::Send, params)
    }

    /// Presses keys by name.
    fn keys(&self, name: &str, keys: &[&str]) -> Result<(), Error> {
        let (name, keys) = (name.to_owned(), names(keys));
        unit(self, Method::Keys, &KeysParams { name, keys })
    }

    /// Resizes a terminal.
    fn resize(&self, name: &str, rows: u16, cols: u16) -> Result<(), Error> {
        let name = name.to_owned();
        unit(self, Method::Resize, &ResizeParams { name, rows, cols })
    }

    /// Reports a terminal's state.
    fn state(&self, name: &str, report: &Reported) -> Result<TerminalInfo, Error> {
        let (name, report) = (name.to_owned(), report.clone());
        typed(self, Method::State, &StateParams { name, report })
    }

    /// Ends a terminal's process tree.
    fn kill(&self, name: &str) -> Result<TerminalInfo, Error> {
        typed(self, Method::Kill, &target(name))
    }

    /// Removes an exited terminal.
    fn rm(&self, name: &str) -> Result<TerminalInfo, Error> {
        typed(self, Method::Rm, &target(name))
    }

    /// Kills, then removes a terminal; one already gone is fine.
    fn close(&self, name: &str) -> Result<(), Error> {
        for method in [Method::Kill, Method::Rm] {
            match unit(self, method, &target(name)) {
                Err(e) if e.code() == Some(ErrorCode::NoSuchTerminal) => return Ok(()),
                done => done?,
            }
        }
        Ok(())
    }

    /// Blocks until a state or output matches, the program exits or the wait's timeout passes.
    fn wait(&self, params: &WaitParams) -> Result<WaitResult, Error> {
        typed(self, Method::Wait, params)
    }

    /// Waits up to `secs` for one of the states `until`.
    fn wait_state(&self, name: &str, until: &[&str], secs: f64) -> Result<WaitResult, Error> {
        let (name, until) = (name.to_owned(), names(until));
        let timeout = Some(secs);
        self.wait(&WaitParams {
            name,
            until,
            timeout,
            ..WaitParams::default()
        })
    }

    /// Waits up to `secs` for a visible line matching `pattern`.
    fn wait_output(&self, name: &str, pattern: &str, secs: f64) -> Result<WaitResult, Error> {
        let (name, output) = (name.to_owned(), Some(pattern.to_owned()));
        let timeout = Some(secs);
        self.wait(&WaitParams {
            name,
            output,
            timeout,
            ..WaitParams::default()
        })
    }

    /// Waits up to `secs` for an ACP terminal's session to be ready for a prompt.
    fn ready(&self, name: &str, secs: f64) -> Result<AcpSession, Error> {
        let done = self.wait_output(name, "ACP session ready", secs)?;
        let outcome = done.outcome;
        let why = || Error::Bad(format!("{name}: no ACP session ({outcome:?})"));
        done.terminal.session.ok_or_else(why)
    }

    /// Prompts an ACP terminal: it reports `working`, then the bridge gets the framed text.
    fn prompt(&self, name: &str, text: &str) -> Result<(), Error> {
        let state = "working".to_owned();
        self.state(
            name,
            &Reported {
                state,
                ..Reported::default()
            },
        )?;
        to_bridge(self, name, &Input::Prompt(text.to_owned()))
    }

    /// Answers an ACP terminal's permission request (else its oldest) with an option id or kind.
    fn approve(&self, name: &str, option: &str, request: Option<&str>) -> Result<(), Error> {
        let (option, request) = (option.to_owned(), request.map(str::to_owned));
        to_bridge(self, name, &Input::Approve { option, request })
    }

    /// Cancels an ACP terminal's turn.
    fn cancel(&self, name: &str) -> Result<(), Error> {
        to_bridge(self, name, &Input::Cancel)
    }

    /// Switches an ACP terminal's model.
    fn model(&self, name: &str, model: &str) -> Result<(), Error> {
        let (name, model) = (name.to_owned(), model.to_owned());
        unit(self, Method::Model, &ModelParams { name, model })
    }

    /// The agents shed knows.
    fn agents(&self) -> Result<Vec<AgentInfo>, Error> {
        typed(self, Method::Agents, &Map::new())
    }

    /// Agent `id`'s limit and auth rules applied to `text`.
    fn matched(&self, id: &str, text: &str) -> Result<Matched, Error> {
        let (id, text) = (id.to_owned(), text.to_owned());
        typed(self, Method::AgentsMatch, &MatchParams { id, text })
    }

    /// Each configured machine's held link.
    fn links(&self) -> Result<BTreeMap<String, Link>, Error> {
        typed(self, Method::MachineLinks, &Map::new())
    }

    /// Runs a program without a terminal.
    fn exec(&self, params: &ExecParams) -> Result<ExecResult, Error> {
        typed(self, Method::Exec, params)
    }

    /// The machine's health.
    fn stats(&self, params: &StatsParams) -> Result<MachineStats, Error> {
        typed(self, Method::MachineStats, params)
    }

    /// A file's bytes.
    fn file_read(&self, params: &FileParams) -> Result<Vec<u8>, Error> {
        typed(self, Method::FileRead, params)
    }

    /// Replaces a file with `params.data`.
    fn file_write(&self, params: &FileParams) -> Result<(), Error> {
        unit(self, Method::FileWrite, params)
    }

    /// The accounts, of one provider or all, per provider `default` first.
    fn accounts(&self, provider: Option<&str>) -> Result<Vec<Account>, Error> {
        let provider = provider.map(str::to_owned);
        typed(self, Method::AccountLs, &AccountsParams { provider })
    }

    /// Makes an account, or adopts a folder for it.
    fn account_add(&self, params: &AccountAdd) -> Result<Account, Error> {
        typed(self, Method::AccountAdd, params)
    }

    /// Starts the account's login in a redacted terminal.
    fn account_login(&self, provider: &str, name: &str) -> Result<Login, Error> {
        typed(self, Method::AccountLogin, &account(provider, name))
    }

    /// Reads the account's status and usage now.
    fn account_status(&self, provider: &str, name: &str) -> Result<Account, Error> {
        typed(self, Method::AccountStatus, &account(provider, name))
    }

    /// Logs the account out and removes it.
    fn account_rm(&self, provider: &str, name: &str) -> Result<(), Error> {
        unit(self, Method::AccountRm, &account(provider, name))
    }

    /// Emits an ACP terminal's structured event.
    fn emit(&self, params: &EmitParams) -> Result<(), Error> {
        unit(self, Method::Emit, params)
    }

    /// Would the server hand over to a binary.
    fn handover_check(&self, params: &HandoverCheckParams) -> Result<Handover, Error> {
        typed(self, Method::HandoverCheck, params)
    }

    /// Hands the server over to its own binary.
    fn handover(&self, params: &HandoverParams) -> Result<Handover, Error> {
        typed(self, Method::Handover, params)
    }
}

fn to_bridge<C: Calls + ?Sized>(calls: &C, name: &str, input: &Input) -> Result<(), Error> {
    let (name, text) = (name.to_owned(), frame(input));
    calls.send(&SendParams {
        name,
        text,
        ..SendParams::default()
    })
}
