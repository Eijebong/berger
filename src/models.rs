#[derive(Debug, serde::Serialize, sqlx::Type)]
#[sqlx(type_name = "task")]
pub struct Task {
    pub id: String,
    pub status: String,
    pub name: String,
}

#[derive(Debug, serde::Serialize)]
pub struct TaskGroup {
    pub id: String,
    pub tasks: Vec<Task>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub repo_org: String,
    pub repo_name: String,
    pub git_ref: String,
    pub source: String,
}
