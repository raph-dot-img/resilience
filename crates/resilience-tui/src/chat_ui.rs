//! "Local messages" (ADR 0018): conversations with nearby people, without
//! the Internet. The interface only reads and writes the files of the
//! account's message store (`resilience_core::messages`); the per-user
//! program `resilience-messages` does the networking.
//!
//! - **Conversations**: one line per person; `n` writes to a new address,
//!   `p` lists the people heard on the network, `m` shows my address.
//! - **A conversation**: every message with what is known about it
//!   (waiting, sent, delivered, FAILED; from them, verified or not).
//!   `w` writes, `c` gives up on a waiting message, `a` names the person.
//!
//! Names heard on the network are claims and are shown as `"name"?` until
//! the user gives the address a name of their own.

use std::time::{SystemTime, UNIX_EPOCH};

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;
use resilience_core::messages::{
    self, Conversation, MAX_NAME_CHARS, Name, Peer, Record, Status, Store,
};

use super::{App, Effect, Level, Modal, View, clamp_selection, is_activate, is_back, nav};

/// Longest message typed in the dialog, in characters.
pub const MAX_TYPED_CHARS: usize = 1000;
/// The lists are read again this often while a messages screen is shown.
const RELOAD_TICKS: u64 = 8;

/// A conversation with the name shown for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatRow {
    pub conv: Conversation,
    pub name: Name,
}

/// Someone heard on the network, with the name shown for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NearbyRow {
    pub peer: Peer,
    pub name: Name,
}

/// What the text dialog is asking for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChatInput {
    /// The address of someone to write to.
    Address,
    /// A message for `peer`.
    Text { peer: String },
    /// The user's own name for `peer`.
    ContactName { peer: String },
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

impl App {
    pub(super) fn chat_store(&self) -> Store {
        Store::new(messages::store_dir(&self.home))
    }

    fn chat_rows(&self) -> (Vec<ChatRow>, Status) {
        let store = self.chat_store();
        let (contacts, peers) = (store.contacts(), store.peers());
        let rows = messages::conversations(&store.records())
            .into_iter()
            .map(|conv| ChatRow {
                name: Name::of(&conv.peer, &contacts, &peers),
                conv,
            })
            .collect();
        (rows, store.status(now_secs()))
    }

    fn nearby_rows(&self) -> Vec<NearbyRow> {
        let store = self.chat_store();
        let contacts = store.contacts();
        let peers = store.peers();
        peers
            .iter()
            .map(|p| NearbyRow {
                name: Name::of(&p.address, &contacts, &peers),
                peer: p.clone(),
            })
            .collect()
    }

    fn chat_view(&self, peer: &str) -> View {
        let store = self.chat_store();
        let records: Vec<Record> = store
            .records()
            .into_iter()
            .filter(|r| r.peer == peer)
            .collect();
        let name = Name::of(peer, &store.contacts(), &store.peers());
        // Start on the newest message.
        let sel = records.len().checked_sub(1);
        View::Chat {
            peer: peer.to_string(),
            name,
            records,
            list: ListState::default().with_selected(sel),
        }
    }

    pub(super) fn open_chats(&mut self) -> Option<View> {
        let (rows, status) = self.chat_rows();
        let sel = (!rows.is_empty()).then_some(0);
        Some(View::Chats {
            rows,
            status,
            list: ListState::default().with_selected(sel),
        })
    }

    /// Read the store again into every messages screen. The selection
    /// stays on the same line, or follows the newest message when it was
    /// on it.
    pub(super) fn reload_chats(&mut self) {
        let (rows, status) = self.chat_rows();
        let nearby = self.nearby_rows();
        let store = self.chat_store();
        let (contacts, peers, all) = (store.contacts(), store.peers(), store.records());
        for v in &mut self.stack {
            match v {
                View::Chats {
                    rows: r,
                    status: s,
                    list,
                } => {
                    let keep = list
                        .selected()
                        .and_then(|i| r.get(i))
                        .map(|x| x.conv.peer.clone());
                    *r = rows.clone();
                    *s = status.clone();
                    match keep.and_then(|p| r.iter().position(|x| x.conv.peer == p)) {
                        Some(i) => list.select(Some(i)),
                        None => clamp_selection(list, r.len()),
                    }
                }
                View::Nearby { rows: r, list } => {
                    *r = nearby.clone();
                    clamp_selection(list, r.len());
                }
                View::Chat {
                    peer,
                    name,
                    records,
                    list,
                } => {
                    let at_end = list.selected().is_none_or(|i| i + 1 >= records.len());
                    *records = all.iter().filter(|x| &x.peer == peer).cloned().collect();
                    *name = Name::of(peer, &contacts, &peers);
                    if at_end {
                        list.select(records.len().checked_sub(1));
                    } else {
                        clamp_selection(list, records.len());
                    }
                }
                _ => {}
            }
        }
    }

    /// From the main loop's tick: keep a visible messages screen current.
    pub(super) fn chat_tick(&mut self) {
        if self.tick.is_multiple_of(RELOAD_TICKS)
            && matches!(
                self.stack.last(),
                Some(View::Chats { .. } | View::Nearby { .. } | View::Chat { .. })
            )
        {
            self.reload_chats();
        }
    }

    fn ask(&mut self, kind: ChatInput, value: String) {
        self.modal = Some(Modal::ChatInput {
            kind,
            value,
            error: None,
        });
    }

    fn show_my_address(&mut self, status: &Status) {
        let body = match status.address() {
            Some(a) => format!(
                "Your address for local messages:\n\n  {}\n\nGive it to the people who \
                 should be able to write to you, and compare it with them in person or by \
                 voice: the address, not a name, is what identifies you. It is not a \
                 secret.{}",
                messages::format_address(a),
                match status {
                    Status::Running { .. } => "",
                    Status::Stopped { .. } =>
                        "\n\n[!!] Messaging is not running for your account right now: \
                         nothing is sent or received until it is (log out and in again).",
                }
            ),
            None => "Messaging has not started for your account yet, so you have no address. \
                     Log out and log in again; if this message stays, ask your \
                     administrator whether local messaging is enabled on this computer."
                .into(),
        };
        self.modal = Some(Modal::Message {
            title: "MY ADDRESS".into(),
            body,
            error: false,
        });
    }

    fn show_person(&mut self, peer: &str, name: &Name) {
        let who = match name {
            Name::Contact(n) => format!("You named this address: {n}"),
            Name::Claimed(n) => format!(
                "This address says its name is \"{n}\". Anyone can announce any name: \
                 press a to give it a name of your own once you have compared the \
                 address with the person."
            ),
            Name::Unknown => "This address has no name yet. Press a to give it one.".into(),
        };
        self.modal = Some(Modal::Message {
            title: "PERSON".into(),
            body: format!(
                "Address:\n\n  {}\n\n{who}\n\nA message marked NOT VERIFIED could not be \
                 checked against this address's key (yet): treat it with care.",
                messages::format_address(peer)
            ),
            error: false,
        });
    }

    /// Keys of the three messages screens.
    pub(super) fn chat_key(&mut self, mut view: View, key: KeyEvent) -> (Option<View>, Effect) {
        let mut next: Option<View> = None;
        match &mut view {
            View::Chats { rows, status, list } => match key.code {
                KeyCode::Char('n') => self.ask(ChatInput::Address, String::new()),
                KeyCode::Char('p') => {
                    let rows = self.nearby_rows();
                    let sel = (!rows.is_empty()).then_some(0);
                    next = Some(View::Nearby {
                        rows,
                        list: ListState::default().with_selected(sel),
                    });
                }
                KeyCode::Char('m') => {
                    let s = status.clone();
                    self.show_my_address(&s);
                }
                KeyCode::Char('r') => {
                    (*rows, *status) = self.chat_rows();
                    clamp_selection(list, rows.len());
                }
                _ if is_activate(key) => {
                    if let Some(r) = list.selected().and_then(|i| rows.get(i)) {
                        next = Some(self.chat_view(&r.conv.peer));
                    }
                }
                _ if is_back(key) => return (None, Effect::None),
                _ => nav(list, rows.len(), key),
            },
            View::Nearby { rows, list } => {
                let selected = list.selected().and_then(|i| rows.get(i)).cloned();
                match key.code {
                    KeyCode::Char('a') => {
                        if let Some(r) = selected {
                            self.ask_name(&r.peer.address, &r.name);
                        }
                    }
                    KeyCode::Char('i') => {
                        if let Some(r) = selected {
                            self.show_person(&r.peer.address, &r.name);
                        }
                    }
                    KeyCode::Char('r') => {
                        *rows = self.nearby_rows();
                        clamp_selection(list, rows.len());
                    }
                    _ if is_activate(key) => {
                        if let Some(r) = selected {
                            next = Some(self.chat_view(&r.peer.address));
                        }
                    }
                    _ if is_back(key) => return (None, Effect::None),
                    _ => nav(list, rows.len(), key),
                }
            }
            View::Chat {
                peer,
                name,
                records,
                list,
            } => {
                let selected = list.selected().and_then(|i| records.get(i)).cloned();
                match key.code {
                    KeyCode::Char('w') | KeyCode::Enter => {
                        self.ask(ChatInput::Text { peer: peer.clone() }, String::new());
                    }
                    KeyCode::Char('a') => {
                        let (p, n) = (peer.clone(), name.clone());
                        self.ask_name(&p, &n);
                    }
                    KeyCode::Char('i') => {
                        let (p, n) = (peer.clone(), name.clone());
                        self.show_person(&p, &n);
                    }
                    KeyCode::Char('c') => match selected {
                        Some(r) if r.state.pending() => match self.chat_store().cancel(&r.id) {
                            Ok(()) => self.notify(
                                Level::Info,
                                "The message will not be sent again (if it has not arrived already).",
                            ),
                            Err(e) => {
                                self.notify(Level::Error, format!("could not cancel: {e}"));
                            }
                        },
                        Some(_) => self.notify(
                            Level::Info,
                            "Only a message that is still waiting or sent can be cancelled.",
                        ),
                        None => {}
                    },
                    _ if is_back(key) => return (None, Effect::None),
                    _ => nav(list, records.len(), key),
                }
            }
            _ => {}
        }
        if let Some(n) = next {
            self.stack.push(view);
            self.stack.push(n);
            return (None, Effect::None);
        }
        (Some(view), Effect::None)
    }

    fn ask_name(&mut self, peer: &str, name: &Name) {
        let current = match name {
            Name::Contact(n) => n.clone(),
            _ => String::new(),
        };
        self.ask(
            ChatInput::ContactName {
                peer: peer.to_string(),
            },
            current,
        );
    }

    /// Keys of the text dialog.
    pub(super) fn chat_input_key(
        &mut self,
        kind: ChatInput,
        mut value: String,
        error: Option<String>,
        key: KeyEvent,
    ) -> Effect {
        let max = match kind {
            ChatInput::Address => 48,
            ChatInput::Text { .. } => MAX_TYPED_CHARS,
            ChatInput::ContactName { .. } => MAX_NAME_CHARS,
        };
        let mut error = error;
        match key.code {
            KeyCode::Esc => return Effect::None,
            KeyCode::Enter => match self.chat_submit(&kind, &value) {
                Ok(()) => return Effect::None,
                Err(e) => error = Some(e),
            },
            KeyCode::Backspace => {
                value.pop();
                error = None;
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if value.chars().count() < max {
                    value.push(c);
                }
                error = None;
            }
            _ => {}
        }
        self.modal = Some(Modal::ChatInput { kind, value, error });
        Effect::None
    }

    fn chat_submit(&mut self, kind: &ChatInput, value: &str) -> Result<(), String> {
        let store = self.chat_store();
        match kind {
            ChatInput::Address => {
                let addr = messages::parse_address(value).ok_or(
                    "An address has 32 characters: digits and the letters a to f. Check it \
                     with the person.",
                )?;
                if store.status(now_secs()).address() == Some(addr.as_str()) {
                    return Err("This is your own address.".into());
                }
                let v = self.chat_view(&addr);
                self.push(v);
                self.ask(ChatInput::Text { peer: addr }, String::new());
                Ok(())
            }
            ChatInput::Text { peer } => {
                let status = store.status(now_secs());
                store
                    .queue(peer, value, status.address())
                    .map_err(|e| e.to_string())?;
                // Never "sent" here: the program has not even taken it yet.
                match status {
                    Status::Running { .. } => self.notify(
                        Level::Info,
                        "Message saved. What happens to it is shown beside it.",
                    ),
                    Status::Stopped { .. } => self.notify(
                        Level::Warn,
                        "Message saved, but messaging is not running for your account: it \
                         will be sent when it runs again.",
                    ),
                }
                self.reload_chats();
                Ok(())
            }
            ChatInput::ContactName { peer } => {
                store.set_contact(peer, value).map_err(|e| e.to_string())?;
                self.reload_chats();
                Ok(())
            }
        }
    }
}
