//! The agent loop's control flow: how many steps a turn takes, when it stops, and when it goes on.
//!
//! A step is one model call. The model either answers, which ends the turn unless the user sent
//! more guidance meanwhile, or calls tools, which run before the next step. The last step offers
//! no tools, so a long run still ends in an answer. Cancelling stops the turn at once.
//!
//! What a step does (stream the reply, run the tools, save) is the app's, behind [`Steps`]; this
//! is the loop around it, the same for a chat, a headless run, a worker or an eval.

use async_trait::async_trait;

/// What one model call did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reply {
    /// It answered, with no tool calls.
    Answered,
    /// It called tools, which [`Steps::run_calls`] runs next.
    CalledTools,
    /// The user stopped the turn while it ran.
    Cancelled,
}

// async-trait's generated methods trip this lint on current clippy; the futures are awaited.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait Steps: Send {
    /// One model call. On the `last` step no tools may be offered.
    async fn call_model(&mut self, step: usize, last: bool) -> Result<Reply, String>;
    /// Runs the tool calls of the step just made; `true` when the user stopped the turn.
    async fn run_calls(&mut self, step: usize) -> Result<bool, String>;
    /// Whether the user sent guidance during the step, which earns another step.
    fn has_new_guidance(&self) -> bool;
}

/// Runs a turn of at most `max_steps` model calls. Returns whether the user cancelled it.
pub async fn run<S: Steps + ?Sized>(steps: &mut S, max_steps: usize) -> Result<bool, String> {
    let max_steps = max_steps.max(1);
    for step in 0..max_steps {
        let last = step + 1 == max_steps;
        match steps.call_model(step, last).await? {
            Reply::Cancelled => return Ok(true),
            Reply::Answered if steps.has_new_guidance() && !last => continue,
            Reply::Answered => return Ok(false),
            Reply::CalledTools => {
                if steps.run_calls(step).await? {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scripted turn: what each model call does, and what happened.
    struct Script {
        replies: Vec<Reply>,
        guidance_after: Option<usize>,
        cancel_tools_at: Option<usize>,
        log: Vec<String>,
    }

    #[async_trait]
    impl Steps for Script {
        async fn call_model(&mut self, step: usize, last: bool) -> Result<Reply, String> {
            self.log
                .push(format!("model {step}{}", if last { " last" } else { "" }));
            Ok(self.replies.get(step).copied().unwrap_or(Reply::Answered))
        }
        async fn run_calls(&mut self, step: usize) -> Result<bool, String> {
            self.log.push(format!("tools {step}"));
            Ok(self.cancel_tools_at == Some(step))
        }
        fn has_new_guidance(&self) -> bool {
            self.guidance_after.is_some_and(|after| {
                self.log.iter().filter(|l| l.starts_with("model")).count() == after
            })
        }
    }

    fn script(replies: &[Reply]) -> Script {
        Script {
            replies: replies.to_vec(),
            guidance_after: None,
            cancel_tools_at: None,
            log: Vec::new(),
        }
    }

    async fn run_script(s: &mut Script, max: usize) -> bool {
        run(s, max).await.unwrap()
    }

    #[tokio::test]
    async fn tools_run_between_model_calls_until_an_answer() {
        use Reply::*;
        let mut s = script(&[CalledTools, CalledTools, Answered]);
        assert!(!run_script(&mut s, 10).await);
        assert_eq!(
            s.log,
            ["model 0", "tools 0", "model 1", "tools 1", "model 2"]
        );
    }

    #[tokio::test]
    async fn the_last_step_is_marked_and_ends_the_turn() {
        let mut s = script(&[Reply::CalledTools; 5]);
        assert!(!run_script(&mut s, 3).await);
        // The third call is the last; whatever it does, there is no fourth.
        assert!(s.log.contains(&"model 2 last".to_string()));
        assert_eq!(s.log.iter().filter(|l| l.starts_with("model")).count(), 3);
    }

    #[tokio::test]
    async fn guidance_during_an_answer_earns_another_step() {
        let mut s = script(&[Reply::Answered, Reply::Answered]);
        s.guidance_after = Some(1);
        assert!(!run_script(&mut s, 5).await);
        assert_eq!(s.log, ["model 0", "model 1"]);
        // Not past the last step.
        let mut last = script(&[Reply::Answered]);
        last.guidance_after = Some(1);
        run_script(&mut last, 1).await;
        assert_eq!(last.log, ["model 0 last"]);
    }

    #[tokio::test]
    async fn cancelling_stops_at_once() {
        let mut s = script(&[Reply::Cancelled, Reply::Answered]);
        assert!(run_script(&mut s, 5).await);
        assert_eq!(s.log, ["model 0"]);
        let mut during_tools = script(&[Reply::CalledTools, Reply::Answered]);
        during_tools.cancel_tools_at = Some(0);
        assert!(run_script(&mut during_tools, 5).await);
        assert_eq!(during_tools.log, ["model 0", "tools 0"]);
    }
}
