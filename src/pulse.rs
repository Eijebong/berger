use anyhow::{Context, Result};
use futures_lite::stream::StreamExt;
use lapin::{options::*, types::FieldTable, Connection, ConnectionProperties};

pub async fn start_pulse_handler() -> Result<()> {
    let addr = std::env::var("AMQP_ADDR").unwrap_or_else(|_| "amqp://127.0.0.1:5672/%2f".into());

    let conn = Connection::connect(&addr, ConnectionProperties::default()).await?;

    tracing::info!("CONNECTED");

    let channel_a = conn.create_channel().await?;
    channel_a
        .queue_declare(
            "berger",
            QueueDeclareOptions::default(),
            FieldTable::default(),
        )
        .await?;
    channel_a
        .queue_bind(
            "berger",
            "exchange/taskcluster-queue/v1/task-pending",
            "#",
            QueueBindOptions::default(),
            FieldTable::default(),
        )
        .await?;
    channel_a
        .queue_bind(
            "berger",
            "exchange/taskcluster-queue/v1/task-running",
            "#",
            QueueBindOptions::default(),
            FieldTable::default(),
        )
        .await?;
    channel_a
        .queue_bind(
            "berger",
            "exchange/taskcluster-queue/v1/task-completed",
            "#",
            QueueBindOptions::default(),
            FieldTable::default(),
        )
        .await?;
    channel_a
        .queue_bind(
            "berger",
            "exchange/taskcluster-queue/v1/task-failed",
            "#",
            QueueBindOptions::default(),
            FieldTable::default(),
        )
        .await?;
    channel_a
        .queue_bind(
            "berger",
            "exchange/taskcluster-queue/v1/task-exception",
            "#",
            QueueBindOptions::default(),
            FieldTable::default(),
        )
        .await?;

    let mut consumer = channel_a
        .basic_consume(
            "berger",
            "test",
            BasicConsumeOptions::default(),
            FieldTable::default(),
        )
        .await?;

    while let Some(Ok(msg)) = consumer.next().await {
        if let Err(e) = handle_message(msg.exchange.as_str(), msg.data.clone()).await {
            tracing::error!("Error while handling message {}.", e);
        }
        msg.ack(BasicAckOptions::default()).await?;
    }

    Ok(())
}

async fn handle_message(exchange: &str, data: Vec<u8>) -> Result<()> {
    let payload = String::from_utf8(data)?;
    let payload: serde_json::Value = serde_json::from_str(&payload)?;
    let mut conn = crate::db::POOL.get().unwrap().acquire().await?;

    let task_id = payload
        .pointer("/status/taskId")
        .context("status/taskId not present")?
        .as_str()
        .context("taskId is not a string")?;
    let group_id = payload
        .pointer("/status/taskGroupId")
        .context("status/taskGroupId not present")?
        .as_str()
        .context("groupId is not a string")?;

    let created_at = chrono::DateTime::parse_from_rfc3339(
        payload["status"]["runs"][0]["scheduled"]
            .as_str()
            .unwrap_or("1970-01-01T00:00:00Z"),
    )?;

    let extra_info = get_extra_task_info(task_id).await?;

    // Decision task
    if group_id == task_id {
        sqlx::query!(
            "INSERT INTO task_groups(id, created_at, repo_org, repo_name, git_ref, source) VALUES($1, $2, $3, $4, $5, $6) ON CONFLICT DO NOTHING",
            group_id,
            created_at,
            extra_info.repo_org,
            extra_info.repo_name,
            extra_info.git_ref,
            extra_info.source,
        )
        .execute(&mut conn)
        .await?;
    }

    match exchange {
        "exchange/taskcluster-queue/v1/task-pending" => {
            sqlx::query!("INSERT INTO tasks(id, status, task_group, name) VALUES($1, $2, $3, $4) ON CONFLICT(id) DO UPDATE SET status=$2", task_id, "pending", group_id, extra_info.name).execute(&mut conn).await?;
        }
        "exchange/taskcluster-queue/v1/task-running" => {
            sqlx::query!("INSERT INTO tasks(id, status, task_group, name) VALUES($1, $2, $3, $4) ON CONFLICT(id) DO UPDATE SET status=$2", task_id, "running", group_id, extra_info.name).execute(&mut conn).await?;
        }
        "exchange/taskcluster-queue/v1/task-completed" => {
            sqlx::query!("INSERT INTO tasks(id, status, task_group, name) VALUES($1, $2, $3, $4) ON CONFLICT(id) DO UPDATE SET status=$2", task_id, "completed", group_id, extra_info.name).execute(&mut conn).await?;
        }
        "exchange/taskcluster-queue/v1/task-failed" => {
            sqlx::query!("INSERT INTO tasks(id, status, task_group, name) VALUES($1, $2, $3, $4) ON CONFLICT(id) DO UPDATE SET status=$2", task_id, "failed", group_id, extra_info.name).execute(&mut conn).await?;
        }
        "exchange/taskcluster-queue/v1/task-exception" => {
            sqlx::query!("INSERT INTO tasks(id, status, task_group, name) VALUES($1, $2, $3, $4) ON CONFLICT(id) DO UPDATE SET status=$2", task_id, "exception", group_id, extra_info.name).execute(&mut conn).await?;
        }
        _ => {}
    }
    Ok(())
}

pub struct TaskInfo {
    pub name: String,
    pub git_ref: Option<String>,
    pub repo_org: Option<String>,
    pub repo_name: Option<String>,
    pub source: String,
}

async fn get_extra_task_info(task_id: &str) -> Result<TaskInfo> {
    let queue = taskcluster::Queue::new(&**crate::BASE_URL)?;
    let task = queue.task(task_id).await?;
    let name = task
        .pointer("/metadata/name")
        .context("metadata/name is missing")?
        .as_str()
        .context("task name is not a string")?;
    let source = task
        .pointer("/metadata/source")
        .context("metadata/source is missing")?
        .as_str()
        .context("task source is not a string")?;
    let (repo_org, repo_name) = if let Some(source) = source.strip_prefix("https://github.com/") {
        let mut source_parts = source.split('/');
        (source_parts.next(), source_parts.next())
    } else {
        (None, None)
    };

    let git_ref = task
        .pointer("/tags/git_ref")
        .and_then(|v| v.as_str())
        .map(|v| v.to_string());

    Ok(TaskInfo {
        name: name.to_string(),
        git_ref,
        repo_org: repo_org.map(Into::into),
        repo_name: repo_name.map(Into::into),
        source: source.to_string(),
    })
}
