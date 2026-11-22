//! The plan panel (design: "Kalango Remote · Pedir ao agente (com aprovação)"): a floating panel over the window
//! with the agent's steps and who approves each, the limits, and Cancel / Adjust the plan / Approve and connect.

use gtk::{prelude::*, Align};
use std::{cell::RefCell, rc::Rc};

use crate::{
    i18n::{tr, trf},
    plan::{Approval, Decision, Plan},
    ui,
};

pub struct PlanPanel {
    pub root: gtk::Box,
    answer: RefCell<Option<Box<dyn FnOnce(Decision)>>>,
}

impl PlanPanel {
    /// Builds the panel; `answer` runs once with what the person chose (also when the panel is closed).
    pub fn new(plan: &Plan, answer: impl FnOnce(Decision) + 'static) -> Rc<PlanPanel> {
        let root = ui::vbox(14, "kr-plan");
        root.set_halign(Align::End);
        root.set_valign(Align::Start);
        root.set_margin_top(63);
        root.set_margin_end(14);
        root.set_size_request(560, -1);

        let head = ui::hbox(8, "");
        head.append(&ui::icon("spark", "k_accent", 16));
        let title = ui::ellip(&trf("Agent · plan for {device}", &[("device", plan.device.clone())]), "kr-plan-title");
        title.set_hexpand(true);
        head.append(&title);
        head.append(&ui::badge(tr("waiting for you"), "gold").0);
        let close = ui::icon_button("fechar", "k_muted", tr("Close"));
        close.add_css_class("small");
        head.append(&close);
        root.append(&head);

        if !plan.request.is_empty() {
            let row = ui::hbox(0, "");
            let bubble = ui::vbox(0, "kr-plan-request");
            bubble.append(&ui::para(&plan.request, "kr-body"));
            bubble.set_halign(Align::End);
            bubble.set_hexpand(true);
            row.append(&bubble);
            root.append(&row);
        }
        root.append(&ui::para(tr("I will do it like this. Nothing happens there without your approval and theirs:"), "kr-plan-intro"));

        let steps = ui::vbox(8, "");
        for (i, s) in plan.steps.iter().enumerate() {
            let row = ui::hbox(10, "kr-plan-step");
            let mark = ui::label(&(i + 1).to_string(), "kr-plan-mark");
            mark.set_xalign(0.5);
            mark.set_valign(Align::Center);
            row.append(&mark);
            let text = ui::vbox(2, "");
            text.set_hexpand(true);
            text.set_valign(Align::Center);
            text.append(&ui::para(&s.title, "kr-row-title"));
            if !s.detail.is_empty() {
                text.append(&ui::para(&s.detail, "kr-row-sub"));
            }
            row.append(&text);
            match s.approval {
                Approval::You => row.append(&ui::label(tr("approve"), "kr-tag accent")),
                Approval::Them => row.append(&ui::label(tr("they confirm"), "kr-tag gold")),
                Approval::None => {}
            }
            steps.append(&row);
        }
        root.append(&steps);

        let limits = ui::hbox(8, "kr-plan-limits");
        let shield = ui::icon("escudo", "k_accent", 16);
        shield.set_valign(Align::Start);
        limits.append(&shield);
        let text = if plan.limits.is_empty() {
            tr("Limits: only what these steps do. Esc Esc stops everything, here or there.").to_owned()
        } else {
            trf("Limits: {limits}. Esc Esc stops everything, here or there.", &[("limits", plan.limits.trim_end_matches('.').to_owned())])
        };
        limits.append(&ui::para(&text, "kr-plan-limits-text"));
        root.append(&limits);

        let actions = ui::hbox(8, "");
        let cancel = ui::button(tr("Cancel"), None, "");
        let adjust = ui::button(tr("Adjust the plan"), None, "ghost muted");
        let approve = ui::button(tr("Approve and connect"), Some(("check", "k_accent_fg")), "primary tall");
        actions.append(&cancel);
        actions.append(&adjust);
        actions.append(&ui::spacer());
        actions.append(&approve);
        root.append(&actions);

        let panel = Rc::new(PlanPanel { root, answer: RefCell::new(Some(Box::new(answer))) });
        for (b, d) in [(&cancel, Decision::Cancelled), (&close, Decision::Cancelled), (&adjust, Decision::Adjust), (&approve, Decision::Approved)] {
            let p = Rc::downgrade(&panel);
            b.connect_clicked(move |_| {
                if let Some(p) = p.upgrade() {
                    p.decide(d);
                }
            });
        }
        panel
    }

    pub fn decide(&self, d: Decision) {
        self.root.set_visible(false);
        if let Some(a) = self.answer.borrow_mut().take() {
            a(d);
        }
    }
}
