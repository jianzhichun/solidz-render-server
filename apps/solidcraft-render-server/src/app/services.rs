//! The UI's file pickers, answered by the host (design §1.8): every "save to file" and "open a
//! file" the UI asks for becomes an `ask_save` / `ask_open`, and its picker future
//! (`crates/ui-egui/src/file_picker.rs:14`, polled by `file_picker::tick` every pass, `:123-128`)
//! resolves when the host's `answer` comes. A picker program (`pick_program`, the slicer) is
//! not offered: `None` answers as a cancel (`file_picker.rs:73-80`). Everything else a desktop host offers (show in
//! folder, downloads, updates, keychain, agent HTTP, slicer hand-off) stays `None`.
//!
//! A save's answer is a staging place of the host's: the host delivers the file to the user's own
//! folder once the work ran (`ask_done`) and says where it went. The UI's own message about it
//! ("已导出：<path>") would name the staging place, so it is not shown; an error names the file
//! by its name only ([`Asks::shown_status`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};

// `crates/ui-egui/src/lib.rs:201` re-exports the private module's type.
use solidcraft_ui_egui::{FilePickerFuture, Services};

use crate::domain::wire::ServerMsg;

/// The asks waiting for the host, and the ones a pass has just handed to the UI.
#[derive(Default)]
struct Table {
    next: u32,
    waiting: HashMap<u32, Slot>,
    /// Answers the UI took this pass: `(ask, a path was given)`; `ask_done` follows the pass.
    delivered: Vec<(u32, bool)>,
    /// The save places the host gave, newest last, at most [`PLACES_KEPT`]: a background save
    /// may report one long after its pass.
    places: Vec<String>,
}

/// Save places remembered for the status line.
const PLACES_KEPT: usize = 64;

#[derive(Default)]
struct Slot {
    answer: Option<Option<String>>,
    waker: Option<Waker>,
}

/// The asks of this process. Lives on the frame loop's thread with the UI (`Services` holds
/// closures that are not `Send`, `crates/ui-egui/src/lib.rs:487-540`).
#[derive(Clone, Default)]
pub struct Asks {
    table: Rc<RefCell<Table>>,
}

impl Asks {
    /// The `Services` whose pickers ask the host through `send`.
    pub fn services(&self, send: Rc<dyn Fn(ServerMsg)>) -> Services {
        let (open, file, save) = (self.clone(), self.clone(), self.clone());
        let (send_open, send_file, send_save) = (send.clone(), send.clone(), send);
        Services {
            pick_open: Some(Box::new(move |exts: &[&str]| open.ask(&*send_open, |ask| ServerMsg::AskOpen { ask, exts: owned(exts) }))),
            // One kind of file (the agent chat's attachments): the host offers the same chooser.
            pick_file: Some(Box::new(move |_label: &str, exts: &[&str]| file.ask(&*send_file, |ask| ServerMsg::AskOpen { ask, exts: owned(exts) }))),
            pick_save: Some(Box::new(move |name: &str, exts: &[&str]| {
                let future = save.ask(&*send_save, |ask| ServerMsg::AskSave { ask, name: name.to_string(), exts: owned(exts) });
                save.remember_place(future)
            })),
            ..Services::default()
        }
    }

    /// The host answered `ask` (`None`: cancelled). An answer to an ask that is not waiting
    /// (answered twice, or never asked) is ignored.
    pub fn answer(&self, ask: u32, path: Option<String>) {
        let waker = {
            let mut table = self.table.borrow_mut();
            // Already answered but not yet taken by the UI's poll: the first answer stands.
            let Some(slot) = table.waiting.get_mut(&ask).filter(|s| s.answer.is_none()) else { return };
            slot.answer = Some(path);
            slot.waker.take()
        };
        // `file_picker::Repaint` (`file_picker.rs:22-28`): the context repaints. Outside the borrow.
        if let Some(waker) = waker {
            waker.wake();
        }
    }

    /// How the UI's status line reads a message (`SolveApp::shown_status`): one naming a save place
    /// the host gave is not shown when it says the file was written (the host says where it went),
    /// and names the file alone when it is an error.
    pub fn shown_status(&self) -> Box<dyn Fn(&str, bool) -> Option<String>> {
        let table = self.table.clone();
        Box::new(move |text: &str, error: bool| shown(&table.borrow().places, text, error))
    }

    /// A save's answer, remembered as a place of the host's once it comes.
    fn remember_place(&self, answer: FilePickerFuture) -> FilePickerFuture {
        let table = self.table.clone();
        Box::pin(async move {
            let path = answer.await;
            if let Some(path) = &path {
                let places = &mut table.borrow_mut().places;
                places.push(path.clone());
                if places.len() > PLACES_KEPT {
                    places.remove(0);
                }
            }
            path
        })
    }

    /// The answers the UI took since the last call, for `ask_done` after the pass that ran them.
    pub fn take_delivered(&self) -> Vec<(u32, bool)> {
        std::mem::take(&mut self.table.borrow_mut().delivered)
    }

    fn ask(&self, send: &dyn Fn(ServerMsg), msg: impl FnOnce(u32) -> ServerMsg) -> FilePickerFuture {
        let ask = {
            let mut table = self.table.borrow_mut();
            table.next += 1;
            let ask = table.next;
            table.waiting.insert(ask, Slot::default());
            ask
        };
        send(msg(ask));
        Box::pin(Answer { ask, table: self.table.clone() })
    }
}

/// `text` as the user is to read it, given the save places of the host: see [`Asks::shown_status`].
fn shown(places: &[String], text: &str, error: bool) -> Option<String> {
    let Some(place) = places.iter().rev().find(|p| text.contains(p.as_str())) else { return Some(text.to_string()) };
    if !error {
        return None;
    }
    let name = place.rsplit(['/', '\\']).next().unwrap_or(place);
    Some(text.replace(place.as_str(), name))
}

fn owned(exts: &[&str]) -> Vec<String> {
    exts.iter().map(|e| e.trim_start_matches('.').to_string()).collect()
}

/// One ask's picker future.
struct Answer {
    ask: u32,
    table: Rc<RefCell<Table>>,
}

impl Future for Answer {
    type Output = Option<String>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<String>> {
        let mut table = self.table.borrow_mut();
        let Some(slot) = table.waiting.get_mut(&self.ask) else { return Poll::Ready(None) };
        match slot.answer.take() {
            Some(path) => {
                table.waiting.remove(&self.ask);
                table.delivered.push((self.ask, path.is_some()));
                Poll::Ready(path)
            }
            None => {
                slot.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An ask goes to the host at once; its future waits until the answer, which is reported
    /// as delivered once (for `ask_done`); a second answer changes nothing.
    #[test]
    fn a_save_ask_waits_for_the_hosts_answer() {
        let asks = Asks::default();
        let sent = Rc::new(RefCell::new(Vec::new()));
        let log = sent.clone();
        let services = asks.services(Rc::new(move |m: ServerMsg| log.borrow_mut().push(m)));
        let mut future = (services.pick_save.as_ref().unwrap())("part.step", &["step", ".stp"]);
        assert_eq!(*sent.borrow(), vec![ServerMsg::AskSave { ask: 1, name: "part.step".into(), exts: vec!["step".into(), "stp".into()] }]);
        let mut cx = Context::from_waker(Waker::noop());
        assert!(future.as_mut().poll(&mut cx).is_pending());
        asks.answer(1, Some("/staging/part.step".into()));
        asks.answer(1, None);
        assert_eq!(future.as_mut().poll(&mut cx), Poll::Ready(Some("/staging/part.step".into())));
        assert_eq!(asks.take_delivered(), vec![(1, true)]);
        assert!(asks.take_delivered().is_empty());
    }

    /// The host's save place is never shown: a message that the file was written is left out (the
    /// host says where it went), an error names the file alone; other messages are as they are.
    #[test]
    fn a_save_place_of_the_host_is_not_shown() {
        let asks = Asks::default();
        let services = asks.services(Rc::new(|_| {}));
        let mut future = (services.pick_save.as_ref().unwrap())("x.step", &["step"]);
        let place = r"C:\Users\u\AppData\plugins-data\solidz\staging\export-1\x.step";
        asks.answer(1, Some(place.into()));
        assert_eq!(future.as_mut().poll(&mut Context::from_waker(Waker::noop())), Poll::Ready(Some(place.into())));
        let shown = asks.shown_status();
        assert_eq!(shown(&format!("已导出：{place}"), false), None);
        assert_eq!(shown(&format!("{place}: 拒绝访问"), true).as_deref(), Some("x.step: 拒绝访问"));
        assert_eq!(shown("已打开 风扇", false).as_deref(), Some("已打开 风扇"));
        assert_eq!(
            super::shown(&["/tmp/s/export-2/a b.stl".into()], "/tmp/s/export-2/a b.stl: no space", true).as_deref(),
            Some("a b.stl: no space")
        );
    }
}
