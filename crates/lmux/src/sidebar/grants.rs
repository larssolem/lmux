use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, Label, Orientation};

use crate::state::{AnchorGrantView, SharedAppState};

pub(super) fn grant_row(grant: AnchorGrantView, state: SharedAppState) -> gtk4::Widget {
    let row = GtkBox::new(Orientation::Vertical, 4);
    row.add_css_class("lmux-sidebar__grant-row");

    let requester = grant
        .requester
        .name
        .as_deref()
        .unwrap_or(grant.requester.id.as_str());
    let state_text = if grant.pending { "pending" } else { "active" };
    let mut text = format!("{requester} · {:?} · {state_text}", grant.scope);
    if let Some(reason) = &grant.reason {
        text.push_str(" · ");
        text.push_str(reason);
    }
    let label = Label::new(Some(&text));
    label.set_xalign(0.0);
    label.set_wrap(true);
    row.append(&label);

    let buttons = GtkBox::new(Orientation::Horizontal, 4);
    buttons.set_halign(Align::End);
    if grant.pending {
        let allow_once = Button::with_label("Allow once");
        let allow_timed = Button::with_label("Allow 10m");
        let deny = Button::with_label("Deny");
        deny.add_css_class("destructive-action");

        let state_once = state.clone();
        let once_id = grant.grant_id;
        allow_once.connect_clicked(move |_| {
            if let Err(err) = state_once
                .borrow_mut()
                .decide_grant(once_id, lmux_bus::GrantDecision::AllowOnce)
            {
                tracing::warn!(error = %err, "grant allow-once failed");
            }
        });

        let state_timed = state.clone();
        let timed_id = grant.grant_id;
        allow_timed.connect_clicked(move |_| {
            let expires_at_unix_seconds = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_secs() + 600)
                .unwrap_or(600);
            if let Err(err) = state_timed.borrow_mut().decide_grant(
                timed_id,
                lmux_bus::GrantDecision::AllowUntil {
                    expires_at_unix_seconds,
                },
            ) {
                tracing::warn!(error = %err, "grant allow-timed failed");
            }
        });

        let state_deny = state;
        let deny_id = grant.grant_id;
        deny.connect_clicked(move |_| {
            if let Err(err) = state_deny
                .borrow_mut()
                .decide_grant(deny_id, lmux_bus::GrantDecision::Deny)
            {
                tracing::warn!(error = %err, "grant deny failed");
            }
        });

        buttons.append(&allow_once);
        buttons.append(&allow_timed);
        buttons.append(&deny);
    } else if grant.active {
        let revoke = Button::with_label("Revoke");
        revoke.add_css_class("destructive-action");
        let state_revoke = state;
        let revoke_id = grant.grant_id;
        revoke.connect_clicked(move |_| {
            if let Err(err) = state_revoke
                .borrow_mut()
                .decide_grant(revoke_id, lmux_bus::GrantDecision::Revoke)
            {
                tracing::warn!(error = %err, "grant revoke failed");
            }
        });
        buttons.append(&revoke);
    }

    row.append(&buttons);
    row.upcast()
}
