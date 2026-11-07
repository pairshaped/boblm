//! Claude's complete live-task snapshots, independent of turn results and tool output.
use boblm_protocol::{
    Event,
    jobs::{BackgroundJob, JobSnapshot},
};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Default)]
pub(crate) struct BackgroundTasks {
    agent_keys: HashMap<String, String>,
    snapshot: JobSnapshot,
}
impl BackgroundTasks {
    pub(crate) fn event(&mut self, value: &Value) -> Option<Event> {
        if value["type"] != "system" {
            return None;
        }
        if value["subtype"] == "task_started" && value["task_type"] == "local_agent" {
            if let (Some(task), Some(tool)) =
                (value["task_id"].as_str(), value["tool_use_id"].as_str())
                && self.agent_keys.len() < boblm_tuning::BACKGROUND_JOB_SCAN_LIMIT
            {
                self.agent_keys.insert(task.into(), tool.into());
            }
            return None;
        }
        if value["subtype"] != "background_tasks_changed" {
            return None;
        }
        let tasks = value["tasks"]
            .as_array()
            .filter(|tasks| tasks.len() <= boblm_tuning::BACKGROUND_JOB_SCAN_LIMIT);
        let parsed = tasks.and_then(|tasks| {
            tasks
                .iter()
                .filter(|task| task["task_type"] != "local_agent" && task["ambient"] != true)
                .map(|task| {
                    Some(BackgroundJob {
                        id: task["task_id"].as_str()?.to_owned(),
                        command: task["description"].as_str()?.to_owned(),
                        agent_key: task["parent_task_id"].as_str().map(|id| {
                            self.agent_keys
                                .get(id)
                                .cloned()
                                .unwrap_or_else(|| id.to_owned())
                        }),
                    })
                })
                .collect::<Option<Vec<_>>>()
        });
        match parsed {
            Some(tasks) => {
                self.snapshot = JobSnapshot {
                    tasks,
                    available: true,
                }
            }
            None => self.snapshot.available = false,
        }
        Some(Event::BackgroundJobs {
            snapshot: self.snapshot.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn claude_background_jobs_belong_to_the_spawning_agent_and_replace_after_completion() {
        let mut tasks = BackgroundTasks::default();
        tasks.event(&json!({"type":"system","subtype":"task_started","task_type":"local_agent","task_id":"agent-task","tool_use_id":"agent-call"}));
        let event = tasks.event(&json!({"type":"system","subtype":"background_tasks_changed","tasks":[
            {"task_id":"shell","task_type":"local_bash","description":"cargo test","parent_task_id":"agent-task"},
            {"task_id":"land","task_type":"local_bash","description":"bin/land"},
            {"task_id":"ambient","task_type":"local_bash","description":"watch","ambient":true}
        ]})).unwrap();
        let Event::BackgroundJobs { snapshot } = event else {
            panic!("jobs")
        };
        assert!(snapshot.available);
        assert_eq!(snapshot.tasks.len(), 2);
        assert_eq!(snapshot.tasks[0].agent_key.as_deref(), Some("agent-call"));
        assert!(tasks.event(&json!({"type":"result"})).is_none());
        let Event::BackgroundJobs { snapshot } = tasks
            .event(&json!({"type":"system","subtype":"background_tasks_changed","tasks":[]}))
            .unwrap()
        else {
            panic!("jobs")
        };
        assert!(snapshot.available && snapshot.tasks.is_empty());
    }
}
