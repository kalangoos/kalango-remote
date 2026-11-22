//! The agent's plan for a remote task, shown for approval before anything happens.
//!
//! The contract with KalangoOS's agent: it runs `kalango-remote agent-plan FILE` (or `agent-plan -` with the JSON
//! on stdin) and waits. The app shows the plan panel; the command ends when the person decides, printing the
//! decision as one JSON line (`{"decision":"approved"|"cancelled"|"adjust", ...}`) and exiting 0, 1 or 3.
//!
//! ```json
//! {"device": "Notebook da mãe", "request": "conecte no notebook da minha mãe e instale a impressora",
//!  "target": "https://…trycloudflare.com/#k=…",
//!  "steps": [{"title": "Conectar no Notebook da mãe", "detail": "via Cloudflare", "approval": "you"},
//!            {"title": "Adicionar a impressora", "detail": "pode pedir a senha dela", "approval": "them"}],
//!  "limits": "só ver e controlar a janela Configurações; sem arquivos"}
//! ```
//! `target` is optional (a code, link or `host:port`); without it the saved computer called `device` is used.
//! `approval` is `you` (asked of this person), `them` (asked of whoever is at the other computer) or `none`.
//! Everything in it is text from outside: lengths are capped and control characters dropped.

use serde::Deserialize;
use serde_json::json;

const MAX_STEPS: usize = 12;
const MAX_TITLE: usize = 120;
const MAX_DETAIL: usize = 160;
const MAX_REQUEST: usize = 400;
const MAX_LIMITS: usize = 300;
pub const MAX_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Approval {
    You,
    Them,
    #[default]
    None,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Step {
    pub title: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub approval: Approval,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Plan {
    pub device: String,
    #[serde(default)]
    pub request: String,
    #[serde(default)]
    pub target: String,
    pub steps: Vec<Step>,
    #[serde(default)]
    pub limits: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Approved,
    Cancelled,
    Adjust,
}

impl Decision {
    pub fn exit_status(self) -> i32 {
        match self {
            Decision::Approved => 0,
            Decision::Cancelled => 1,
            Decision::Adjust => 3,
        }
    }
    fn word(self) -> &'static str {
        match self {
            Decision::Approved => "approved",
            Decision::Cancelled => "cancelled",
            Decision::Adjust => "adjust",
        }
    }
}

fn clean(s: &str, max: usize) -> String {
    let one_line: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let t = one_line.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() <= max {
        t
    } else {
        format!("{}…", t.chars().take(max - 1).collect::<String>())
    }
}

impl Plan {
    pub fn parse(json: &str) -> Result<Plan, String> {
        if json.len() > MAX_BYTES {
            return Err("the plan is too big".into());
        }
        let mut p: Plan = serde_json::from_str(json).map_err(|e| format!("not a plan: {e}"))?;
        p.device = clean(&p.device, 80);
        p.request = clean(&p.request, MAX_REQUEST);
        p.target = p.target.trim().to_owned();
        p.limits = clean(&p.limits, MAX_LIMITS);
        p.steps.retain(|s| !s.title.trim().is_empty());
        p.steps.truncate(MAX_STEPS);
        for s in &mut p.steps {
            s.title = clean(&s.title, MAX_TITLE);
            s.detail = clean(&s.detail, MAX_DETAIL);
        }
        if p.device.is_empty() {
            return Err("the plan has no device".into());
        }
        if p.steps.is_empty() {
            return Err("the plan has no steps".into());
        }
        Ok(p)
    }

    /// The line the agent reads: what was decided and whether the connection to the device was started.
    pub fn answer(&self, d: Decision, connecting: bool) -> String {
        json!({"decision": d.word(), "device": self.device, "connecting": connecting}).to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"{"device":"Notebook da mãe","request":"instale a impressora","steps":[
        {"title":"Conectar","detail":"via Cloudflare","approval":"you"},
        {"title":"Imprimir","approval":"them"},{"title":"Encerrar"}]}"#;

    #[test]
    fn a_plan_reads_its_steps_and_who_approves_each() {
        let p = Plan::parse(GOOD).unwrap();
        assert_eq!(p.device, "Notebook da mãe");
        assert_eq!(p.steps.iter().map(|s| s.approval).collect::<Vec<_>>(), [Approval::You, Approval::Them, Approval::None]);
        assert_eq!(p.steps[1].detail, "");
    }

    #[test]
    fn what_cannot_be_shown_is_refused_and_outside_text_is_tamed() {
        assert!(Plan::parse("{}").is_err());
        assert!(Plan::parse(r#"{"device":"x","steps":[]}"#).is_err());
        assert!(Plan::parse(r#"{"device":"x","steps":[{"title":"  "}]}"#).is_err());
        assert!(Plan::parse(r#"{"device":"  ","steps":[{"title":"a"}]}"#).is_err());
        assert!(Plan::parse(&"x".repeat(MAX_BYTES + 1)).is_err());
        let long = "a".repeat(500);
        let p = Plan::parse(&format!(r#"{{"device":"d","steps":[{{"title":"{long}\n\u0007b","approval":"you"}}]}}"#)).unwrap();
        assert_eq!(p.steps[0].title.chars().count(), MAX_TITLE);
        assert!(!p.steps[0].title.contains('\n') && p.steps[0].title.ends_with('…'));
        let many: Vec<String> = (0..30).map(|i| format!(r#"{{"title":"p{i}"}}"#)).collect();
        assert_eq!(Plan::parse(&format!(r#"{{"device":"d","steps":[{}]}}"#, many.join(","))).unwrap().steps.len(), MAX_STEPS);
    }

    #[test]
    fn the_answer_says_what_was_decided() {
        let p = Plan::parse(GOOD).unwrap();
        assert_eq!(p.answer(Decision::Approved, true), r#"{"connecting":true,"decision":"approved","device":"Notebook da mãe"}"#);
        assert_eq!((Decision::Approved.exit_status(), Decision::Cancelled.exit_status(), Decision::Adjust.exit_status()), (0, 1, 3));
    }
}
