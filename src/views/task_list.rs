use crate::models::TaskGroup;
use anyhow::Result;

use poem::{handler, session::Session, web::Html};

use std::collections::HashSet;

use serde::Serialize;

use crate::views::utils::{gather_tc_scopes, HtmlOrRedirect};
use crate::{db, get_context_for, BaseContext};
use askama::Template;

#[derive(Serialize)]
struct ComputedTaskGroup<'a> {
    group: &'a TaskGroup,
    status: &'a str,
    start: &'a str,
    end: &'a str,
}

#[derive(Template)]
#[template(path = "index.html")]
struct TaskListContext<'a> {
    base: BaseContext<'a>,
    groups: Vec<ComputedTaskGroup<'a>>,
    failed: bool,
}

#[handler]
pub async fn root(req: &poem::Request, session: &Session) -> Result<HtmlOrRedirect<String>> {
    let scopes = gather_tc_scopes(session).await?;

    let allowed_repos = scopes
        .iter()
        .filter_map(|scope| scope.strip_prefix("berger:get-repo:"))
        .collect::<HashSet<_>>();

    let mut conn = db::POOL.get().unwrap().acquire().await.unwrap();
    let only_failed = req.uri().query() == Some("failed");

    let groups = if !only_failed {
        sqlx::query_as_unchecked!(TaskGroup,
            "SELECT task_groups.id,
                    task_groups.created_at,
                    task_groups.git_ref,
                    task_groups.repo_org,
                    task_groups.repo_name,
                    task_groups.source,
                    array_agg(row(tasks.id, tasks.status, tasks.name)::task) as tasks
             FROM task_groups INNER JOIN tasks on task_groups.id=tasks.task_group GROUP BY task_groups.id ORDER BY task_groups.created_at DESC LIMIT 500").fetch_all(&mut conn).await?
    } else {
        sqlx::query_as_unchecked!(TaskGroup,
            "SELECT task_groups.id,
                    task_groups.created_at,
                    task_groups.git_ref,
                    task_groups.repo_org,
                    task_groups.repo_name,
                    task_groups.source,
                    array_agg(row(tasks.id, tasks.status, tasks.name)::task) as tasks
             FROM task_groups INNER JOIN tasks on task_groups.id=tasks.task_group GROUP BY task_groups.id HAVING 'failed' = ANY(array_agg(tasks.status)) or 'exception' = ANY(array_agg(tasks.status)) ORDER BY task_groups.created_at DESC LIMIT 500").fetch_all(&mut conn).await?
    };

    let allow_all_repos = allowed_repos.contains("*");
    let groups = groups
        .iter()
        .filter(|group| {
            if allow_all_repos {
                return true;
            }
            let repo_name = format!("{}/{}", group.repo_org, group.repo_name);
            allowed_repos.contains(repo_name.as_str())
        })
        .map(compute_taskgroup_status)
        .collect::<Vec<_>>();

    let tpl = TaskListContext {
        base: get_context_for("index", session),
        groups,
        failed: only_failed,
    };

    Ok(Html(tpl.render().unwrap()).into())
}

fn get_commit_bounds_from_source(source: &str) -> (&str, &str) {
    if !source.starts_with("https://github.com/") {
        return ("", "");
    }

    let commit_range = source.split('/').last().unwrap();

    if !commit_range.contains("...") {
        return (commit_range, "");
    }

    let mut parts = commit_range.split("...");
    let start = parts.next().unwrap();
    let end = parts.next().unwrap();

    (start, end)
}

fn compute_taskgroup_status(group: &TaskGroup) -> ComputedTaskGroup {
    let mut status = "completed";
    let mut has_pending = false;
    let (start, end) = get_commit_bounds_from_source(&group.source);

    for task in &group.tasks {
        if task.status == "failed" || task.status == "exception" {
            status = "failed";
            break;
        }

        if task.status == "pending" {
            has_pending = true;
        }

        if task.status == "running" {
            status = "running";
        }
    }

    if status == "completed" && has_pending {
        status = "pending";
    }

    ComputedTaskGroup {
        group,
        status,
        start,
        end,
    }
}
