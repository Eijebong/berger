use crate::models::TaskGroup;
use anyhow::Result;

use poem::web::Redirect;
use poem::{handler, session::Session, web::Html};

use std::collections::HashSet;

use serde::Serialize;
use taskcluster::Credentials;

use crate::views::utils::HtmlOrRedirect;
use crate::{db, get_context_for, BASE_URL, TEMPLATES};

#[derive(Serialize)]
struct ComputedTaskGroup<'a> {
    group: &'a TaskGroup,
    status: &'a str,
    start: &'a str,
    end: &'a str,
}

#[handler]
pub async fn root(req: &poem::Request, session: &Session) -> Result<HtmlOrRedirect<String>> {
    let mut client = taskcluster::ClientBuilder::new(&**BASE_URL);
    if let Some(creds) = session.get::<Credentials>("credentials") {
        client = client.credentials(creds);
    }
    let tc_auth = taskcluster::Auth::new(client)?;
    let mut scopes = tc_auth.currentScopes().await;
    if scopes.is_err() {
        session.remove("credentials");
        return Ok(Redirect::see_other("/").into());
    }

    assert!(scopes.is_ok());
    let scopes = scopes.unwrap();

    let allowed_repos = scopes["scopes"]
        .as_array()
        .map(|scopes| {
            scopes
                .iter()
                .filter_map(|scope| {
                    let name = scope.as_str()?;
                    name.strip_prefix("berger:get-repo:")
                })
                .collect::<HashSet<_>>()
        })
        .unwrap_or_else(HashSet::new);

    let mut context = get_context_for("index", session);
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
        .map(|group| {
            let mut status = "completed";
            let mut has_pending = false;
            let (start, end) = if group.source.starts_with("https://github.com/") {
                let commit_range = group.source.split('/').last().unwrap();
                if commit_range.contains("...") {
                    let mut parts = commit_range.split("...");
                    let start = parts.next().unwrap();
                    let end = parts.next().unwrap();
                    (start, end)
                } else {
                    (commit_range, "")
                }
            } else {
                ("", "")
            };

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
        })
        .collect::<Vec<_>>();

    context.insert("groups", &groups);
    context.insert("failed", &only_failed);

    Ok(Html(TEMPLATES.render("index.html", &context)?).into())
}
