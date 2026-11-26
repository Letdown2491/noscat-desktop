//! Control (0.6): terminals this computer opens on members, terminals
//! members have open here, the access rules that decide who may do what,
//! and the asks waiting on the user. Client calls block, so every one runs
//! on the background executor (AGENTS.md); results come back as updates.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use gpui_kit::gpui::{Context, WeakEntity};
use noscat_client::{AccessAnswer, AccessGroup, AccessOutcome, AccessRule, AccessRules, Event};

use super::Session;
use crate::views::terminal::TerminalView;

/// How many ended sessions Settings lists.
const RECENT: usize = 5;

/// A member asking to do something that asks first (control).
#[derive(Debug, Clone)]
pub struct AccessAsk {
    pub id: u64,
    pub peer: String,
    pub group: AccessGroup,
    pub asked: Instant,
}

/// A terminal a member has open on this computer.
#[derive(Debug, Clone)]
pub struct Hosted {
    pub id: u64,
    pub peer: String,
    pub started: Instant,
}

/// A hosted session that ended, for Settings.
#[derive(Debug, Clone)]
pub struct Ended {
    pub peer: String,
    pub secs: u64,
    pub ended: Instant,
}

/// A terminal window on a member.
struct Viewer {
    view: WeakEntity<TerminalView>,
    peer: String,
    /// The client's session id, once started.
    session: Option<u64>,
}

#[derive(Default)]
pub struct ControlState {
    viewers: Vec<Viewer>,
    /// Output and exits that beat the start result to the main thread.
    early_output: BTreeMap<u64, Vec<Vec<u8>>>,
    early_exit: BTreeMap<u64, String>,
    pub asks: Vec<AccessAsk>,
    pub hosted: Vec<Hosted>,
    pub recent: Vec<Ended>,
    /// Members that refused this computer a terminal, with the reason
    /// (the Devices row greys its Terminal button and says why).
    pub refused: BTreeMap<String, String>,
    /// The rules everyone without their own gets.
    pub defaults: Option<AccessRules>,
    /// The rules that apply to each member.
    pub rules: BTreeMap<String, AccessRules>,
    /// Members with rules of their own.
    pub custom: BTreeSet<String>,
    /// The mesh's STUN servers.
    pub stun: Vec<String>,
}

/// What a rule reads as in a sentence ("Send allowed").
pub fn rule_word(rule: AccessRule) -> &'static str {
    match rule {
        AccessRule::Allow => "allowed",
        AccessRule::Deny => "blocked",
        AccessRule::Ask => "asks",
    }
}

/// "40 s", "12 min", "3 h".
pub fn span(secs: u64) -> String {
    match secs {
        0..=59 => format!("{secs} s"),
        60..=3599 => format!("{} min", secs / 60),
        _ => format!("{} h", secs / 3600),
    }
}

pub fn group_label(group: AccessGroup) -> &'static str {
    match group {
        AccessGroup::Send => "Send me things",
        AccessGroup::Services => "Use my services",
        AccessGroup::Control => "Control this computer",
    }
}

pub fn rule_of(rules: &AccessRules, group: AccessGroup) -> AccessRule {
    match group {
        AccessGroup::Send => rules.send,
        AccessGroup::Services => rules.services,
        AccessGroup::Control => rules.control,
    }
}

impl Session {
    /// Once a client runs: this app answers access asks, so control can
    /// ask instead of refusing; then load the rules.
    pub(super) fn after_start(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else { return };
        cx.spawn(async move |this, cx| {
            cx.background_executor().spawn(async move { client.set_access_prompts(true) }).await;
            let _ = this.update(cx, |s, cx| s.refresh_access(cx));
        })
        .detach();
    }

    /// The client stopped: its hosted sessions and asks went with it.
    pub(super) fn control_stopped(&mut self) {
        self.control.asks.clear();
        if !self.control.hosted.is_empty() {
            self.control.hosted.clear();
            crate::tray::set_hosting(false);
        }
    }

    /// Reload rules, STUN servers, hosted sessions and pending asks.
    pub fn refresh_access(&mut self, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else { return };
        let npubs: Vec<String> = self.roster.iter().map(|d| d.npub.clone()).filter(|n| Some(n) != self.my_npub.as_ref()).collect();
        cx.spawn(async move |this, cx| {
            let (defaults, rules, custom, stun) = cx
                .background_executor()
                .spawn(async move {
                    let defaults = client.access_defaults();
                    let rules: BTreeMap<String, AccessRules> =
                        npubs.iter().filter_map(|n| client.access_of(n).ok().map(|r| (n.clone(), r))).collect();
                    let custom: BTreeSet<String> = client.access_devices().into_iter().map(|(n, _)| n).collect();
                    (defaults, rules, custom, client.mesh_stun())
                })
                .await;
            let _ = this.update(cx, |s, cx| {
                s.control.defaults = Some(defaults);
                s.control.rules = rules;
                s.control.custom = custom;
                s.control.stun = stun;
                cx.notify();
            });
        })
        .detach();
    }

    /// Set a rule (`None` member: everyone without their own; `None`
    /// rule: back to the default).
    pub fn set_access(&mut self, member: Option<String>, group: AccessGroup, rule: Option<AccessRule>, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else { return };
        cx.spawn(async move |this, cx| {
            let done = cx.background_executor().spawn(async move { client.set_access(member.as_deref(), group, rule) }).await;
            let _ = this.update(cx, |s, cx| {
                if let Err(e) = done {
                    s.last_error = Some(format!("Couldn't change access: {e}"));
                }
                s.refresh_access(cx);
            });
        })
        .detach();
    }

    /// Drop a member's own rules: the defaults apply again.
    pub fn reset_access(&mut self, member: String, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else { return };
        cx.spawn(async move |this, cx| {
            let done = cx.background_executor().spawn(async move { client.reset_access(&member) }).await;
            let _ = this.update(cx, |s, cx| {
                if let Err(e) = done {
                    s.last_error = Some(format!("Couldn't reset access: {e}"));
                }
                s.refresh_access(cx);
            });
        })
        .detach();
    }

    /// Answer an ask.
    pub fn answer_access(&mut self, id: u64, answer: AccessAnswer, cx: &mut Context<Self>) {
        self.control.asks.retain(|a| a.id != id);
        cx.notify();
        let Some(client) = self.client.clone() else { return };
        cx.spawn(async move |this, cx| {
            let done = cx.background_executor().spawn(async move { client.answer_access(id, answer) }).await;
            let _ = this.update(cx, |s, cx| {
                // Gone already (timed out, or the member gave up) is fine.
                let _ = done;
                if answer == AccessAnswer::Always {
                    s.refresh_access(cx);
                }
            });
        })
        .detach();
    }

    /// End a terminal a member has open here.
    pub fn end_hosted(&mut self, id: u64, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else { return };
        cx.background_executor()
            .spawn(async move {
                client.end_terminal(id);
            })
            .detach();
    }

    /// Replace the mesh's STUN servers.
    pub fn set_mesh_stun(&mut self, servers: Vec<String>, cx: &mut Context<Self>) {
        let Some(client) = self.client.clone() else { return };
        cx.spawn(async move |this, cx| {
            let done = cx.background_executor().spawn(async move { client.set_mesh_stun(&servers) }).await;
            let _ = this.update(cx, |s, cx| {
                match done {
                    Ok(()) => s.toasts.push(super::Toast::Done("STUN servers saved. Devices use them from their next start.".into())),
                    Err(e) => s.last_error = Some(format!("Couldn't save the STUN servers: {e}")),
                }
                s.refresh_access(cx);
            });
        })
        .detach();
    }

    /// Start the session for a terminal window, once it knows its size.
    pub fn start_terminal(&mut self, view: WeakEntity<TerminalView>, peer: String, cols: u16, rows: u16, cx: &mut Context<Self>) {
        let name = self.peer_name(&peer);
        let Some(client) = self.client.clone() else {
            let _ = view.update(cx, |v, cx| v.ended("This computer isn't connected.".into(), cx));
            return;
        };
        self.control.viewers.retain(|v| v.view.upgrade().is_some());
        self.control.viewers.push(Viewer { view: view.clone(), peer: peer.clone(), session: None });
        cx.spawn(async move |this, cx| {
            let target = peer.clone();
            let started = cx
                .background_executor()
                .spawn(async move { client.start_terminal(&target, cols, rows, "xterm-256color") })
                .await;
            let _ = this.update(cx, |s, cx| match started {
                Ok(handle) => {
                    let id = handle.id();
                    s.control.refused.remove(&peer);
                    if let Some(v) = s.control.viewers.iter_mut().find(|v| v.view == view && v.session.is_none()) {
                        v.session = Some(id);
                    }
                    let output = s.control.early_output.remove(&id).unwrap_or_default();
                    let exit = s.control.early_exit.remove(&id);
                    let _ = view.update(cx, |v, cx| {
                        v.started(handle, cx);
                        for data in output {
                            v.feed(&data, cx);
                        }
                        if let Some(text) = exit {
                            v.ended(text, cx);
                        }
                    });
                    cx.notify();
                }
                Err(e) => {
                    let e = e.to_string();
                    let lower = e.to_lowercase();
                    let text = if lower.contains("not permitted") {
                        let why = format!("{name} doesn't allow terminals from this computer");
                        s.control.refused.insert(peer.clone(), why.clone());
                        format!("{why}.")
                    } else if lower.contains("does not offer") {
                        let why = format!("{name} doesn't offer a terminal");
                        s.control.refused.insert(peer.clone(), why.clone());
                        format!("{why}.")
                    } else if lower.contains("refused") {
                        format!("{name} said no, or nobody answered in time.")
                    } else {
                        s.scrub_npubs(&format!("Couldn't open a terminal on {name}: {}", Session::human_reason(&e)))
                    };
                    s.control.viewers.retain(|v| v.view != view);
                    let _ = view.update(cx, |v, cx| v.ended(text, cx));
                    cx.notify();
                }
            });
        })
        .detach();
    }

    pub(super) fn control_event(&mut self, ev: Event, cx: &mut Context<Self>) {
        match ev {
            Event::AccessRequest { id, peer, group } => {
                if !self.control.asks.iter().any(|a| a.id == id) {
                    self.control.asks.push(AccessAsk { id, peer, group, asked: Instant::now() });
                }
                // It needs an answer within a minute: bring the window up,
                // reopening it when it was closed to the tray.
                cx.defer(crate::views::show_main_window);
            }
            Event::AccessResolved { id, outcome, .. } => {
                self.control.asks.retain(|a| a.id != id);
                if outcome == AccessOutcome::TimedOut {
                    self.toasts.push(super::Toast::Done("An access request expired unanswered.".into()));
                }
            }
            Event::TerminalWaiting { peer } => {
                for v in self.control.viewers.iter().filter(|v| v.peer == peer && v.session.is_none()) {
                    let _ = v.view.update(cx, |v, cx| v.waiting(cx));
                }
            }
            Event::TerminalOpened { id, peer } => {
                let first = self.control.hosted.is_empty();
                self.control.hosted.push(Hosted { id, peer, started: Instant::now() });
                if first {
                    crate::tray::set_hosting(true);
                }
            }
            Event::TerminalClosed { id, peer, secs, error } => {
                self.control.hosted.retain(|h| h.id != id);
                if self.control.hosted.is_empty() {
                    crate::tray::set_hosting(false);
                }
                self.control.recent.insert(0, Ended { peer: peer.clone(), secs, ended: Instant::now() });
                self.control.recent.truncate(RECENT);
                if let Some(e) = error {
                    let msg = format!("{}'s terminal on this computer ended: {}", self.peer_name(&peer), Session::human_reason(&e));
                    self.last_error = Some(self.scrub_npubs(&msg));
                }
            }
            Event::TerminalOutput { session, data } => {
                match self.control.viewers.iter().find(|v| v.session == Some(session)) {
                    Some(v) => {
                        let _ = v.view.update(cx, |v, cx| v.feed(&data, cx));
                    }
                    None => self.control.early_output.entry(session).or_default().push(data),
                }
                // Output repaints only its window.
                return;
            }
            Event::TerminalExited { session, code, error } => {
                let text = match (code, error) {
                    (Some(0), _) => "The session ended.".to_string(),
                    (Some(c), _) => format!("The shell exited with status {c}."),
                    (None, Some(e)) => format!("The connection was lost: {}", Session::human_reason(&e)),
                    (None, None) => "The session ended.".to_string(),
                };
                match self.control.viewers.iter().position(|v| v.session == Some(session)) {
                    Some(i) => {
                        let v = self.control.viewers.remove(i);
                        let _ = v.view.update(cx, |v, cx| v.ended(text, cx));
                    }
                    None => {
                        self.control.early_exit.insert(session, text);
                    }
                }
                return;
            }
            _ => {}
        }
        cx.notify();
    }
}
