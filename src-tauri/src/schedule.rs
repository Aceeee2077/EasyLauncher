use crate::{
    error::{AppError, Result},
    server::manager::Manager,
    storage,
};
use chrono::{Datelike, Local, NaiveTime, TimeZone, Timelike};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeSet, HashMap},
    fs,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use uuid::Uuid;

const MAX_TASKS: usize = 32;
const MAX_HISTORY: usize = 200;
const MAX_CALENDAR_DAYS: u32 = 31;
const MAX_CALENDAR_ENTRIES: usize = 500;
/// A tick that is a little late is normal (the scheduler wakes every 20 s).
/// Anything beyond this window counts as time the application was not running.
const MISSED_GRACE_SECS: i64 = 120;

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
    Cron,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum MissedRun {
    /// Do not run the task if its time passed while the application was closed.
    Skip,
    /// Run the task once on the next tick, then continue with the future schedule.
    /// This is the default for task files written before the policy existed,
    /// which always ran once on the next tick.
    #[default]
    CatchUp,
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
    pub cron: Option<String>,
    #[serde(default)]
    pub missed_run: MissedRun,
    #[serde(default)]
    pub depends_on: Option<String>,
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
    pub cron: Option<String>,
    #[serde(default)]
    pub missed_run: MissedRun,
    #[serde(default)]
    pub depends_on: Option<String>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub keep_days: u32,
    #[serde(default = "enabled")]
    pub enabled: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEntry {
    pub task_id: String,
    pub action: TaskAction,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub task_id: String,
    pub action: TaskAction,
    pub at: String,
    /// `ok`, `failed`, `missed` or `blocked`.
    pub result: String,
    #[serde(default)]
    pub detail: String,
    /// `schedule` or `manual`.
    pub trigger: String,
}

fn parse_time(value: &str) -> Result<NaiveTime> {
    NaiveTime::parse_from_str(value.trim(), "%H:%M").map_err(|_| {
        AppError::user("Choose a daily time in 24-hour HH:MM format, for example 04:30.")
    })
}

fn parse_cron_field(text: &str, label: &str, min: u32, max: u32) -> Result<BTreeSet<u32>> {
    if text.is_empty() {
        return Err(AppError::user(format!(
            "The {label} field of the cron expression is empty."
        )));
    }
    let mut values = BTreeSet::new();
    for item in text.split(',') {
        if item.is_empty() {
            return Err(AppError::user(format!(
                "The {label} field of the cron expression contains an empty list item."
            )));
        }
        let (base, step) = match item.split_once('/') {
            Some((base, step)) => {
                let step: u32 = step.parse().map_err(|_| {
                    AppError::user(format!(
                        "The step in the {label} field must be a whole number."
                    ))
                })?;
                if step == 0 {
                    return Err(AppError::user(format!(
                        "The step in the {label} field must be at least 1."
                    )));
                }
                (base, Some(step))
            }
            None => (item, None),
        };
        let (start, end) = if base == "*" {
            (min, max)
        } else if let Some((start, end)) = base.split_once('-') {
            let start: u32 = start.parse().map_err(|_| {
                AppError::user(format!(
                    "The {label} field must use numbers between {min} and {max}."
                ))
            })?;
            let end: u32 = end.parse().map_err(|_| {
                AppError::user(format!(
                    "The {label} field must use numbers between {min} and {max}."
                ))
            })?;
            (start, end)
        } else {
            let start: u32 = base.parse().map_err(|_| {
                AppError::user(format!(
                    "The {label} field must use numbers between {min} and {max}."
                ))
            })?;
            (start, if step.is_some() { max } else { start })
        };
        if start < min || start > max || end < min || end > max {
            return Err(AppError::user(format!(
                "The {label} field must stay between {min} and {max}."
            )));
        }
        if start > end {
            return Err(AppError::user(format!(
                "The {label} field has a range that ends before it starts."
            )));
        }
        let step = step.unwrap_or(1);
        let mut value = start;
        while value <= end {
            values.insert(value);
            value += step;
        }
    }
    if values.is_empty() {
        return Err(AppError::user(format!(
            "The {label} field of the cron expression has no values."
        )));
    }
    Ok(values)
}

struct CronSpec {
    minutes: BTreeSet<u32>,
    hours: BTreeSet<u32>,
    days: BTreeSet<u32>,
    months: BTreeSet<u32>,
    weekdays: BTreeSet<u32>,
}

fn parse_cron(expression: &str) -> Result<CronSpec> {
    if expression.len() > 128 {
        return Err(AppError::user("The cron expression is too long."));
    }
    let fields: Vec<&str> = expression.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(AppError::user(
            "A cron expression needs five fields: minute hour day month weekday.",
        ));
    }
    let minutes = parse_cron_field(fields[0], "minute", 0, 59)?;
    let hours = parse_cron_field(fields[1], "hour", 0, 23)?;
    let days = parse_cron_field(fields[2], "day", 1, 31)?;
    let months = parse_cron_field(fields[3], "month", 1, 12)?;
    let mut weekdays = parse_cron_field(fields[4], "weekday", 0, 7)?;
    if weekdays.remove(&7) {
        weekdays.insert(0);
    }
    Ok(CronSpec {
        minutes,
        hours,
        days,
        months,
        weekdays,
    })
}

/// Resolves a wall-clock time to the first real instant after `after`.
/// This also handles the repeated hour when clocks move back: both candidate
/// instants are considered, so a rerun in the second pass is not scheduled in
/// the past. Nonexistent times (clocks moving forward) return `None`.
fn resolve_after(
    target: chrono::NaiveDateTime,
    after: chrono::DateTime<Local>,
) -> Option<chrono::DateTime<Local>> {
    match Local.from_local_datetime(&target) {
        chrono::LocalResult::Single(value) => (value > after).then_some(value),
        chrono::LocalResult::Ambiguous(earliest, latest) => {
            if earliest > after {
                Some(earliest)
            } else if latest > after {
                Some(latest)
            } else {
                None
            }
        }
        chrono::LocalResult::None => None,
    }
}

/// Scans forward minute by minute. The day-of-month and weekday fields must
/// both match, so use `*` in one of them for the usual either-or behaviour.
fn next_cron_run(
    expression: &str,
    after: chrono::DateTime<Local>,
) -> Result<chrono::DateTime<Local>> {
    let spec = parse_cron(expression)?;
    let mut candidate = after
        .naive_local()
        .with_second(0)
        .and_then(|value| value.with_nanosecond(0))
        .ok_or_else(|| AppError::user("The current time cannot be used for scheduling."))?
        + chrono::Duration::minutes(1);
    for _ in 0..(366 * 24 * 60) {
        if spec.months.contains(&candidate.month())
            && spec.days.contains(&candidate.day())
            && spec
                .weekdays
                .contains(&candidate.weekday().num_days_from_sunday())
            && spec.hours.contains(&candidate.hour())
            && spec.minutes.contains(&candidate.minute())
        {
            if let Some(next) = resolve_after(candidate, after) {
                return Ok(next);
            }
        }
        candidate += chrono::Duration::minutes(1);
    }
    Err(AppError::user(
        "This cron expression does not have a run within the next year.",
    ))
}

fn next_after(
    input: &TaskInput,
    after: chrono::DateTime<Local>,
) -> Result<chrono::DateTime<Local>> {
    match input.mode {
        TaskMode::Interval => Ok(after + chrono::Duration::minutes(input.every_minutes as i64)),
        TaskMode::Daily => {
            let time = parse_time(input.at_time.as_deref().unwrap_or(""))?;
            let mut date = after.date_naive();
            for _ in 0..3 {
                if let Some(next) = resolve_after(date.and_time(time), after) {
                    return Ok(next);
                }
                date += chrono::Duration::days(1);
            }
            Err(AppError::user(
                "That daily time is not valid in this timezone.",
            ))
        }
        TaskMode::Cron => next_cron_run(input.cron.as_deref().unwrap_or(""), after),
    }
}

fn next_run(input: &TaskInput) -> Result<String> {
    Ok(next_after(input, Local::now())?.to_rfc3339())
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
        TaskMode::Cron => {
            next_cron_run(input.cron.as_deref().unwrap_or(""), Local::now())?;
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

fn validate_dependency(tasks: &[Task], input: &TaskInput, task_id: Option<&str>) -> Result<()> {
    let Some(parent) = input
        .depends_on
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(());
    };
    storage::validate_id(parent)?;
    if Some(parent) == task_id {
        return Err(AppError::user("A scheduled task cannot depend on itself."));
    }
    let mut seen = BTreeSet::new();
    let mut cursor = Some(parent.to_string());
    while let Some(current) = cursor {
        if Some(current.as_str()) == task_id {
            return Err(AppError::user(
                "This dependency would create a loop between scheduled tasks.",
            ));
        }
        if !seen.insert(current.clone()) {
            return Err(AppError::user(
                "The selected tasks already form a dependency loop.",
            ));
        }
        let task = tasks
            .iter()
            .find(|task| task.id == current)
            .ok_or_else(|| AppError::user("The selected dependency no longer exists."))?;
        cursor = task.depends_on.clone().filter(|value| !value.is_empty());
    }
    Ok(())
}

fn input_from_task(task: &Task) -> TaskInput {
    TaskInput {
        action: task.action,
        mode: task.mode,
        every_minutes: task.every_minutes,
        at_time: task.at_time.clone(),
        cron: task.cron.clone(),
        missed_run: task.missed_run,
        depends_on: task.depends_on.clone(),
        command: task.command.clone(),
        message: task.message.clone(),
        keep_days: task.keep_days,
        enabled: task.enabled,
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
    validate_dependency(&tasks, &input, None)?;
    let next_run_at = next_run(&input)?;
    let task = Task {
        id: Uuid::new_v4().to_string(),
        action: input.action,
        mode: input.mode,
        every_minutes: input.every_minutes,
        at_time: input.at_time.map(|t| t.trim().to_string()),
        cron: input
            .cron
            .map(|c| c.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|c| !c.is_empty()),
        missed_run: input.missed_run,
        depends_on: input
            .depends_on
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
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
    validate_dependency(&tasks, &input, Some(task_id))?;
    let task = tasks
        .iter_mut()
        .find(|t| t.id == task_id)
        .ok_or_else(|| AppError::user("This scheduled task no longer exists."))?;
    let next_run_at = next_run(&input)?;
    task.action = input.action;
    task.mode = input.mode;
    task.every_minutes = input.every_minutes;
    task.at_time = input.at_time.map(|t| t.trim().to_string());
    task.cron = input
        .cron
        .map(|c| c.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|c| !c.is_empty());
    task.missed_run = input.missed_run;
    task.depends_on = input
        .depends_on
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
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
    let required_by = tasks
        .iter()
        .filter(|task| task.depends_on.as_deref() == Some(task_id))
        .count();
    if required_by > 0 {
        return Err(AppError::user(format!(
            "{required_by} other scheduled task(s) depend on this task. Remove those dependencies first."
        )));
    }
    let before = tasks.len();
    tasks.retain(|t| t.id != task_id);
    if tasks.len() == before {
        return Err(AppError::user("This scheduled task no longer exists."));
    }
    write(m, id, &tasks)
}

/// Computes the next few run times for an unsaved draft, so the editor can
/// show a checked preview before anything is persisted.
pub fn preview(input: &TaskInput, count: Option<u32>) -> Result<Vec<String>> {
    validate(input)?;
    let count = count.unwrap_or(5).clamp(1, 20) as usize;
    let mut runs = Vec::with_capacity(count);
    let mut cursor = Local::now();
    for _ in 0..count {
        let next = next_after(input, cursor)?;
        runs.push(next.to_rfc3339());
        cursor = next;
    }
    Ok(runs)
}

/// Upcoming runs for enabled tasks, used by the calendar view.
pub fn calendar(m: &Manager, id: &str, days: Option<u32>) -> Result<Vec<CalendarEntry>> {
    let days = days.unwrap_or(7).clamp(1, MAX_CALENDAR_DAYS);
    let now = Local::now();
    let end = now + chrono::Duration::days(i64::from(days));
    let mut entries = Vec::new();
    for task in read(m, id)?.into_iter().filter(|task| task.enabled) {
        let input = input_from_task(&task);
        let mut cursor = now;
        for _ in 0..200 {
            let next = match next_after(&input, cursor) {
                Ok(next) => next,
                Err(_) => break,
            };
            if next > end || entries.len() >= MAX_CALENDAR_ENTRIES {
                break;
            }
            entries.push(CalendarEntry {
                task_id: task.id.clone(),
                action: task.action,
                at: next.to_rfc3339(),
            });
            cursor = next;
        }
        if entries.len() >= MAX_CALENDAR_ENTRIES {
            break;
        }
    }
    entries.sort_by(|a, b| a.at.cmp(&b.at));
    Ok(entries)
}

fn history_path(m: &Manager, id: &str) -> Result<PathBuf> {
    m.metadata(id)?;
    storage::child(&m.dir(id)?, "task-history.json")
}

pub fn history(m: &Manager, id: &str, task_id: Option<&str>) -> Result<Vec<HistoryEntry>> {
    let file = history_path(m, id)?;
    if !file.exists() {
        return Ok(Vec::new());
    }
    storage::no_links(&file)?;
    let mut entries: Vec<HistoryEntry> = serde_json::from_slice(&fs::read(file)?)?;
    if let Some(task_id) = task_id {
        storage::validate_id(task_id)?;
        entries.retain(|entry| entry.task_id == task_id);
    }
    entries.truncate(MAX_HISTORY);
    Ok(entries)
}

fn push_history(m: &Manager, id: &str, entry: HistoryEntry) -> Result<()> {
    let file = history_path(m, id)?;
    let mut entries: Vec<HistoryEntry> = if file.exists() {
        storage::no_links(&file)?;
        serde_json::from_slice(&fs::read(&file)?)?
    } else {
        Vec::new()
    };
    entries.insert(0, entry);
    entries.truncate(MAX_HISTORY);
    storage::write_json(&file, &entries)
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

fn action_key(action: TaskAction) -> &'static str {
    match action {
        TaskAction::Backup => "backup",
        TaskAction::Restart => "restart",
        TaskAction::Start => "start",
        TaskAction::Stop => "stop",
        TaskAction::Command => "command",
        TaskAction::Cleanup => "cleanup",
        TaskAction::Broadcast => "broadcast",
        TaskAction::Maintenance => "maintenance",
    }
}

fn note_outcome(
    m: &Manager,
    id: &str,
    task: &Task,
    result: &str,
    detail: &str,
    touch_run: bool,
    trigger: &str,
) -> Result<()> {
    let at = Local::now().to_rfc3339();
    let mut tasks = read(m, id)?;
    if let Some(stored) = tasks.iter_mut().find(|stored| stored.id == task.id) {
        if touch_run {
            stored.last_run_at = Some(at.clone());
        }
        stored.last_result = Some(result.to_string());
    }
    write(m, id, &tasks)?;
    push_history(
        m,
        id,
        HistoryEntry {
            task_id: task.id.clone(),
            action: task.action,
            at,
            result: result.to_string(),
            detail: detail.to_string(),
            trigger: trigger.to_string(),
        },
    )
}

fn record(m: &Manager, id: &str, task: &Task, result: &Result<()>, trigger: &str) -> Result<()> {
    match result {
        Ok(()) => note_outcome(m, id, task, "ok", "", true, trigger),
        Err(e) => note_outcome(m, id, task, "failed", &e.message, true, trigger),
    }
}

pub async fn run_now(m: &Arc<Manager>, id: &str, task_id: &str) -> Result<()> {
    storage::validate_id(task_id)?;
    let task = read(m, id)?
        .into_iter()
        .find(|t| t.id == task_id)
        .ok_or_else(|| AppError::user("This scheduled task no longer exists."))?;
    let result = execute(m, id, &task).await;
    let _ = record(m, id, &task, &result, "manual");
    if let Err(e) = &result {
        m.notice(
            "taskFailed",
            id,
            Some(format!("{} · {}", action_key(task.action), e.message)),
        );
    }
    m.emit(
        "schedule-updated",
        serde_json::json!({ "id": id, "taskId": task_id }),
    );
    result
}

fn is_missed(next: chrono::DateTime<Local>, now: chrono::DateTime<Local>) -> bool {
    now.signed_duration_since(next).num_seconds() > MISSED_GRACE_SECS
}

/// Runs that are deeper in a dependency chain come later, so a task that is
/// due in the same tick as its prerequisite waits until the prerequisite ran.
fn dependency_depth(task_id: &str, tasks: &[Task], due: &BTreeSet<String>) -> usize {
    let mut depth = 0;
    let mut cursor = task_id;
    let mut seen = BTreeSet::new();
    while let Some(task) = tasks.iter().find(|task| task.id == cursor) {
        let Some(parent) = task
            .depends_on
            .as_deref()
            .filter(|parent| due.contains(*parent))
        else {
            break;
        };
        if !seen.insert(parent.to_string()) {
            break;
        }
        depth += 1;
        cursor = parent;
    }
    depth
}

async fn tick(m: &Arc<Manager>, id: &str) -> Result<()> {
    let now = Local::now();
    let mut tasks = read(m, id)?;
    let mut due = Vec::new();
    let mut missed = Vec::new();
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
        let input = input_from_task(task);
        task.next_run_at = next_run(&input)?;
        let late = is_missed(next, now);
        if late && task.missed_run == MissedRun::Skip {
            missed.push(task.id.clone());
        } else {
            due.push(task.id.clone());
        }
    }
    if due.is_empty() && missed.is_empty() {
        return Ok(());
    }
    // Advance the schedule before running so a long backup never fires twice.
    write(m, id, &tasks)?;
    m.emit("schedule-updated", serde_json::json!({ "id": id }));
    for task_id in missed {
        let Some(task) = tasks.iter().find(|task| task.id == task_id).cloned() else {
            continue;
        };
        m.logger.log(
            "INFO",
            &format!("Scheduled task {task_id} on {id} was missed; skipped by policy."),
        );
        let _ = note_outcome(
            m,
            id,
            &task,
            "missed",
            "The scheduled time passed while EasyLauncher was not running.",
            false,
            "schedule",
        );
        m.emit(
            "schedule-updated",
            serde_json::json!({ "id": id, "taskId": task_id }),
        );
    }
    let due_set: BTreeSet<String> = due.iter().cloned().collect();
    due.sort_by_key(|task_id| dependency_depth(task_id, &tasks, &due_set));
    let mut outcomes: HashMap<String, bool> = HashMap::new();
    for task_id in due {
        let Some(task) = tasks.iter().find(|task| task.id == task_id).cloned() else {
            continue;
        };
        if let Some(parent) = task
            .depends_on
            .as_deref()
            .filter(|parent| due_set.contains(*parent))
        {
            if outcomes.get(parent) != Some(&true) {
                let detail = format!("Dependency {parent} did not complete successfully.");
                m.logger.log(
                    "WARN",
                    &format!("Scheduled task {task_id} on {id} was skipped: {detail}"),
                );
                let _ = note_outcome(m, id, &task, "blocked", &detail, false, "schedule");
                outcomes.insert(task_id.clone(), false);
                m.emit(
                    "schedule-updated",
                    serde_json::json!({ "id": id, "taskId": task_id }),
                );
                continue;
            }
        }
        let result = execute(m, id, &task).await;
        if let Err(e) = &result {
            m.logger
                .log("WARN", &format!("Scheduled task {task_id} on {id}: {e}"));
            m.notice(
                "taskFailed",
                id,
                Some(format!("{} · {}", action_key(task.action), e.message)),
            );
        }
        let _ = record(m, id, &task, &result, "schedule");
        outcomes.insert(task_id.clone(), result.is_ok());
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
            cron: None,
            missed_run: MissedRun::Skip,
            depends_on: None,
            command: None,
            message: None,
            keep_days: 0,
            enabled: true,
        }
    }

    fn id(seed: &str) -> String {
        let value = match seed {
            "a" => 1,
            "b" => 2,
            "c" => 3,
            _ => 4,
        };
        format!("00000000-0000-4000-8000-{value:012}")
    }

    fn task(id: &str, depends_on: Option<&str>) -> Task {
        Task {
            id: id.into(),
            action: TaskAction::Restart,
            mode: TaskMode::Interval,
            every_minutes: 30,
            at_time: None,
            cron: None,
            missed_run: MissedRun::Skip,
            depends_on: depends_on.map(str::to_string),
            command: None,
            message: None,
            keep_days: 0,
            enabled: true,
            next_run_at: Local::now().to_rfc3339(),
            last_run_at: None,
            last_result: None,
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

    #[test]
    fn cron_expressions_are_validated() {
        let mut i = input(TaskAction::Backup, TaskMode::Cron);
        assert!(validate(&i).is_err());
        i.cron = Some("* * * *".into());
        assert!(validate(&i).is_err());
        i.cron = Some("*/0 * * * *".into());
        assert!(validate(&i).is_err());
        i.cron = Some("60 * * * *".into());
        assert!(validate(&i).is_err());
        i.cron = Some("5-1 * * * *".into());
        assert!(validate(&i).is_err());
        i.cron = Some("0 0 31 2 *".into());
        assert!(validate(&i).is_err());
        i.cron = Some("*/15 4 * * *".into());
        assert!(validate(&i).is_ok());
    }

    #[test]
    fn cron_preview_steps_forward() {
        let mut i = input(TaskAction::Backup, TaskMode::Cron);
        i.cron = Some("*/15 * * * *".into());
        let runs = preview(&i, Some(3)).unwrap();
        assert_eq!(runs.len(), 3);
        let mut previous = Local::now();
        for run in runs {
            let at = chrono::DateTime::parse_from_rfc3339(&run)
                .unwrap()
                .with_timezone(&Local);
            assert!(at > previous);
            assert_eq!(at.minute() % 15, 0);
            assert_eq!(at.second(), 0);
            previous = at;
        }
    }

    #[test]
    fn cron_accepts_sunday_as_seven() {
        let spec = parse_cron("0 4 * * 7").unwrap();
        assert!(spec.weekdays.contains(&0));
        assert_eq!(spec.weekdays.len(), 1);
        let ranges = parse_cron("0 4 * * 0-7").unwrap();
        assert_eq!(ranges.weekdays.len(), 7);
    }

    #[test]
    fn daily_preview_stays_ahead() {
        let mut i = input(TaskAction::Restart, TaskMode::Daily);
        i.at_time = Some("04:30".into());
        let runs = preview(&i, Some(3)).unwrap();
        assert_eq!(runs.len(), 3);
        let mut previous = Local::now();
        for run in runs {
            let at = chrono::DateTime::parse_from_rfc3339(&run)
                .unwrap()
                .with_timezone(&Local);
            assert!(at > previous);
            assert_eq!(at.hour(), 4);
            assert_eq!(at.minute(), 30);
            previous = at;
        }
    }

    #[test]
    fn missed_runs_respect_the_grace_window() {
        let now = Local::now();
        assert!(!is_missed(now - chrono::Duration::seconds(60), now));
        assert!(is_missed(now - chrono::Duration::seconds(180), now));
    }

    #[test]
    fn dependency_loops_are_rejected() {
        let (a, b, c, missing) = (id("a"), id("b"), id("c"), id("missing"));
        let tasks = vec![task(&a, None), task(&b, Some(&a)), task(&c, Some(&b))];
        let mut i = input(TaskAction::Restart, TaskMode::Interval);
        i.depends_on = Some(a.clone());
        assert!(validate_dependency(&tasks, &i, Some(a.as_str())).is_err());
        i.depends_on = Some(c.clone());
        assert!(validate_dependency(&tasks, &i, Some(a.as_str())).is_err());
        i.depends_on = Some(missing);
        assert!(validate_dependency(&tasks, &i, None).is_err());
        i.depends_on = Some(b);
        assert!(validate_dependency(&tasks, &i, None).is_ok());
    }

    #[test]
    fn dependencies_run_before_dependants() {
        let (a, b, c) = (id("a"), id("b"), id("c"));
        let tasks = vec![task(&a, None), task(&b, Some(&a)), task(&c, Some(&b))];
        let due: BTreeSet<String> = [a.clone(), b.clone(), c.clone()].into_iter().collect();
        let mut order = vec![a.clone(), b.clone(), c.clone()];
        order.sort_by_key(|task_id| dependency_depth(task_id, &tasks, &due));
        assert_eq!(order, vec![a.clone(), b.clone(), c.clone()]);
        let blocked = dependency_depth(&c, &tasks, &BTreeSet::from([c.clone()]));
        assert_eq!(blocked, 0);
    }
}
