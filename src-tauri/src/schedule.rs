use crate::{
    error::{AppError, Result},
    server::manager::Manager,
    storage,
};
use chrono::{Local, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};
use std::{fs, path::PathBuf, sync::Arc, time::Duration};
use uuid::Uuid;

const MAX_TASKS: usize = 32;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TaskAction {
    Backup,
    Restart,
    Start,
    Stop,
    Command,
    Cleanup,
    Broadcast,
    Maintenance,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TaskMode {
    Interval,
    Daily,
}

fn enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub action: TaskAction,
    pub mode: TaskMode,
    #[serde(default)]
    pub every_minutes: u32,
    #[serde(default)]
    pub at_time: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub keep_days: u32,
    #[serde(default = "enabled")]
    pub enabled: bool,
    pub next_run_at: String,
    #[serde(default)]
    pub last_run_at: Option<String>,
    #[serde(default)]
    pub last_result: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskInput {
    pub action: TaskAction,
    pub mode: TaskMode,
    #[serde(default)]
    pub every_minutes: u32,
    #[serde(default)]
    pub at_time: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub keep_days: u32,
    #[serde(default = "enabled")]
    pub enabled: bool,
}

fn parse_time(value: &str) -> Result<NaiveTime> {
    NaiveTime::parse_from_str(value.trim(), "%H:%M").map_err(|_| {
        AppError::user("Choose a daily time in 24-hour HH:MM format, for example 04:30.")
    })
}

fn validate(input: &TaskInput) -> Result<()> {
    match input.mode {
        TaskMode::Interval => {
            if !(5..=10080).contains(&input.every_minutes) {
                return Err(AppError::user(
                    "Choose an interval between 5 minutes and 7 days.",
                ));
            }
        }
        TaskMode::Daily => {
            parse_time(input.at_time.as_deref().unwrap_or(""))?;
        }
    }
    if input.action == TaskAction::Command {
        let command = input.command.as_deref().unwrap_or("").trim();
        if command.is_empty() || command.chars().count() > 512 {
            return Err(AppError::user(
                "Enter one console command, up to 512 characters.",
            ));
        }
        if command.contains(['\n', '\r', '\0']) {
            return Err(AppError::user("Console commands must fit on one line."));
        }
    }
    if input.action == TaskAction::Cleanup && !(1..=365).contains(&input.keep_days) {
        return Err(AppError::user("Keep log files for between 1 and 365 days."));
    }
    if matches!(
        input.action,
        TaskAction::Broadcast | TaskAction::Maintenance
    ) {
        let message = input.message.as_deref().unwrap_or("").trim();
        if input.action == TaskAction::Broadcast && message.is_empty() {
            return Err(AppError::user(
                "Enter an announcement message, up to 256 characters.",
            ));
        }
        if !message.is_empty()
            && (message.chars().count() > 256 || message.contains(['\n', '\r', '\0']))
        {
            return Err(AppError::user(
                "Announcements must fit on one line, up to 256 characters.",
            ));
        }
    }
    Ok(())
}

fn next_run(input: &TaskInput) -> Result<String> {
    let now = Local::now();
    match input.mode {
        TaskMode::Interval => {
            Ok((now + chrono::Duration::minutes(input.every_minutes as i64)).to_rfc3339())
        }
        TaskMode::Daily => {
            let time = parse_time(input.at_time.as_deref().unwrap_or(""))?;
            let target = now.date_naive().and_time(time);
            let mut next = Local
                .from_local_datetime(&target)
                .earliest()
                .ok_or_else(|| AppError::user("That daily time is not valid in this timezone."))?;
            if next <= now {
                next += chrono::Duration::days(1);
            }
            Ok(next.to_rfc3339())
        }
    }
}

fn path(m: &Manager, id: &str) -> Result<PathBuf> {
    m.metadata(id)?;
    storage::child(&m.dir(id)?, "tasks.json")
}

fn read(m: &Manager, id: &str) -> Result<Vec<Task>> {
    let file = path(m, id)?;
    if !file.exists() {
        return Ok(Vec::new());
    }
    storage::no_links(&file)?;
    let tasks: Vec<Task> = serde_json::from_slice(&fs::read(file)?)?;
    if tasks.len() > MAX_TASKS {
        return Err(AppError::user("This server has too many scheduled tasks."));
    }
    Ok(tasks)
}

fn write(m: &Manager, id: &str, tasks: &[Task]) -> Result<()> {
    storage::write_json(&path(m, id)?, &tasks.to_vec())
}

pub fn list(m: &Manager, id: &str) -> Result<Vec<Task>> {
    let mut tasks = read(m, id)?;
    tasks.sort_by(|a, b| a.next_run_at.cmp(&b.next_run_at));
    Ok(tasks)
}

pub fn create(m: &Manager, id: &str, input: TaskInput) -> Result<Task> {
    validate(&input)?;
    let mut tasks = read(m, id)?;
    if tasks.len() >= MAX_TASKS {
        return Err(AppError::user(
            "A server can have at most 32 scheduled tasks.",
        ));
    }
    let next_run_at = next_run(&input)?;
    let task = Task {
        id: Uuid::new_v4().to_string(),
        action: input.action,
        mode: input.mode,
        every_minutes: input.every_minutes,
        at_time: input.at_time.map(|t| t.trim().to_string()),
        command: input.command.map(|c| c.trim().to_string()),
        message: input.message.map(|m| m.trim().to_string()),
        keep_days: input.keep_days,
        enabled: input.enabled,
        next_run_at,
        last_run_at: None,
        last_result: None,
    };
    tasks.push(task.clone());
    write(m, id, &tasks)?;
    Ok(task)
}

pub fn update(m: &Manager, id: &str, task_id: &str, input: TaskInput) -> Result<Task> {
    storage::validate_id(task_id)?;
    validate(&input)?;
    let mut tasks = read(m, id)?;
    let task = tasks
        .iter_mut()
        .find(|t| t.id == task_id)
        .ok_or_else(|| AppError::user("This scheduled task no longer exists."))?;
    let next_run_at = next_run(&input)?;
    task.action = input.action;
    task.mode = input.mode;
    task.every_minutes = input.every_minutes;
    task.at_time = input.at_time.map(|t| t.trim().to_string());
    task.command = input.command.map(|c| c.trim().to_string());
    task.message = input.message.map(|m| m.trim().to_string());
    task.keep_days = input.keep_days;
    task.enabled = input.enabled;
    task.next_run_at = next_run_at;
    let result = task.clone();
    write(m, id, &tasks)?;
    Ok(result)
}

pub fn delete(m: &Manager, id: &str, task_id: &str) -> Result<()> {
    storage::validate_id(task_id)?;
    let mut tasks = read(m, id)?;
    let before = tasks.len();
    tasks.retain(|t| t.id != task_id);
    if tasks.len() == before {
        return Err(AppError::user("This scheduled task no longer exists."));
    }
    write(m, id, &tasks)
}

fn cleanup(m: &Manager, id: &str, keep_days: u32) -> Result<()> {
    let dir = storage::child(&m.dir(id)?, "logs")?;
    if !dir.is_dir() {
        return Ok(());
    }
    let cutoff = std::time::SystemTime::now()
        - std::time::Duration::from_secs(u64::from(keep_days) * 24 * 60 * 60);
    for entry in fs::read_dir(dir)?.flatten() {
        let file = entry.path();
        storage::no_links(&file)?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !(name.ends_with(".log") || name.ends_with(".log.gz")) {
            continue;
        }
        if entry
            .metadata()?
            .modified()
            .map(|t| t < cutoff)
            .unwrap_or(false)
        {
            fs::remove_file(file)?;
        }
    }
    Ok(())
}

async fn execute(m: &Arc<Manager>, id: &str, task: &Task) -> Result<()> {
    match task.action {
        TaskAction::Backup => crate::backup::create_backup(
            m.clone(),
            id.to_string(),
            crate::models::BackupReason::Scheduled,
        )
        .await
        .map(|_| ()),
        TaskAction::Restart => m.restart(id).await,
        TaskAction::Start => m.start(id).await,
        TaskAction::Stop => m.stop(id).await,
        TaskAction::Command => m.send(id, task.command.as_deref().unwrap_or("")).await,
        TaskAction::Broadcast => {
            let message = task.message.as_deref().unwrap_or("");
            m.send(id, &format!("say {message}")).await
        }
        TaskAction::Maintenance => {
            let message = task.message.as_deref().unwrap_or("").trim();
            if !message.is_empty() && m.runtime(id)?.snapshot().status.active() {
                m.send(id, &format!("say {message}")).await?;
            }
            m.stop(id).await?;
            crate::backup::create_backup(
                m.clone(),
                id.to_string(),
                crate::models::BackupReason::Scheduled,
            )
            .await
            .map(|_| ())
        }
        TaskAction::Cleanup => cleanup(m, id, task.keep_days),
    }
}

fn record(m: &Manager, id: &str, task_id: &str, result: &Result<()>) -> Result<()> {
    let mut tasks = read(m, id)?;
    if let Some(task) = tasks.iter_mut().find(|t| t.id == task_id) {
        task.last_run_at = Some(Local::now().to_rfc3339());
        task.last_result = Some(match result {
            Ok(()) => "ok".into(),
            Err(e) => e.message.clone(),
        });
    }
    write(m, id, &tasks)
}

pub async fn run_now(m: &Arc<Manager>, id: &str, task_id: &str) -> Result<()> {
    storage::validate_id(task_id)?;
    let task = read(m, id)?
        .into_iter()
        .find(|t| t.id == task_id)
        .ok_or_else(|| AppError::user("This scheduled task no longer exists."))?;
    let result = execute(m, id, &task).await;
    let _ = record(m, id, task_id, &result);
    m.emit(
        "schedule-updated",
        serde_json::json!({ "id": id, "taskId": task_id }),
    );
    result
}

async fn tick(m: &Arc<Manager>, id: &str) -> Result<()> {
    let now = Local::now();
    let mut tasks = read(m, id)?;
    let mut due = Vec::new();
    for task in tasks.iter_mut() {
        if !task.enabled {
            continue;
        }
        let next = chrono::DateTime::parse_from_rfc3339(&task.next_run_at)
            .map(|t| t.with_timezone(&Local))
            .unwrap_or(now);
        if next > now {
            continue;
        }
        let input = TaskInput {
            action: task.action,
            mode: task.mode,
            every_minutes: task.every_minutes,
            at_time: task.at_time.clone(),
            command: task.command.clone(),
            message: task.message.clone(),
            keep_days: task.keep_days,
            enabled: task.enabled,
        };
        task.next_run_at = next_run(&input)?;
        due.push(task.id.clone());
    }
    if due.is_empty() {
        return Ok(());
    }
    // Advance the schedule before running so a long backup never fires twice.
    write(m, id, &tasks)?;
    m.emit("schedule-updated", serde_json::json!({ "id": id }));
    for task_id in due {
        let task = match tasks.iter().find(|t| t.id == task_id) {
            Some(task) => task.clone(),
            None => continue,
        };
        let result = execute(m, id, &task).await;
        if let Err(e) = &result {
            m.logger
                .log("WARN", &format!("Scheduled task {task_id} on {id}: {e}"));
        }
        let _ = record(m, id, &task_id, &result);
        m.emit(
            "schedule-updated",
            serde_json::json!({ "id": id, "taskId": task_id }),
        );
    }
    Ok(())
}

pub fn spawn(m: Arc<Manager>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(20)).await;
            if m.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
                continue;
            }
            let ids: Vec<String> = m
                .runtimes
                .lock()
                .map(|r| r.keys().cloned().collect())
                .unwrap_or_default();
            for id in ids {
                if let Err(e) = tick(&m, &id).await {
                    m.logger
                        .log("WARN", &format!("Scheduled tasks for {id} failed: {e}"));
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(action: TaskAction, mode: TaskMode) -> TaskInput {
        TaskInput {
            action,
            mode,
            every_minutes: 30,
            at_time: None,
            command: None,
            message: None,
            keep_days: 0,
            enabled: true,
        }
    }

    #[test]
    fn interval_bounds() {
        let mut i = input(TaskAction::Restart, TaskMode::Interval);
        assert!(validate(&i).is_ok());
        i.every_minutes = 4;
        assert!(validate(&i).is_err());
        i.every_minutes = 60 * 24 * 8;
        assert!(validate(&i).is_err());
    }

    #[test]
    fn daily_requires_hh_mm() {
        let mut i = input(TaskAction::Restart, TaskMode::Daily);
        assert!(validate(&i).is_err());
        i.at_time = Some("25:00".into());
        assert!(validate(&i).is_err());
        i.at_time = Some("04:30".into());
        assert!(validate(&i).is_ok());
        assert!(next_run(&i).is_ok());
    }

    #[test]
    fn command_and_cleanup_validation() {
        let mut i = input(TaskAction::Command, TaskMode::Interval);
        assert!(validate(&i).is_err());
        i.command = Some("say backup soon\nstop".into());
        assert!(validate(&i).is_err());
        i.command = Some("say backup soon".into());
        assert!(validate(&i).is_ok());
        let mut c = input(TaskAction::Cleanup, TaskMode::Interval);
        assert!(validate(&c).is_err());
        c.keep_days = 14;
        assert!(validate(&c).is_ok());
    }

    #[test]
    fn broadcast_and_maintenance_validation() {
        let mut broadcast = input(TaskAction::Broadcast, TaskMode::Daily);
        broadcast.at_time = Some("04:00".into());
        assert!(validate(&broadcast).is_err());
        broadcast.message = Some("Server restarting soon".into());
        assert!(validate(&broadcast).is_ok());
        broadcast.message = Some("line one\nline two".into());
        assert!(validate(&broadcast).is_err());
        broadcast.message = Some("x".repeat(257));
        assert!(validate(&broadcast).is_err());
        let mut window = input(TaskAction::Maintenance, TaskMode::Daily);
        window.at_time = Some("04:00".into());
        assert!(validate(&window).is_ok());
        window.message = Some("Maintenance starting".into());
        assert!(validate(&window).is_ok());
    }

    #[test]
    fn next_run_moves_forward() {
        let i = input(TaskAction::Backup, TaskMode::Interval);
        let next = chrono::DateTime::parse_from_rfc3339(&next_run(&i).unwrap()).unwrap();
        assert!(next > Local::now());
    }
}
