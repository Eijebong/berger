use anyhow::Result;

use poem::web::Query;
use poem::{handler, session::Session, web::Html};
use taskcluster::Hooks;

use std::collections::HashSet;

use serde::Serialize;

use crate::views::utils::{gather_tc_scopes, HtmlOrRedirect};
use crate::{db, get_context_for, BaseContext, GITHUB_TRIGGER_HOOK};
use askama::Template;
use itertools::Itertools;

struct RepoInfo {
    repo_org: String,
    repo_name: String,
    git_ref: String,
    task_id: String,
}

#[derive(Serialize, Clone)]
struct ComputedRepoInfo {
    repo_org: String,
    repo_name: String,
    branches: Vec<(String, String)>,
}

#[derive(Template)]
#[template(path = "repo_list.html")]
struct RepoListContext<'a> {
    base: BaseContext<'a>,
    repos: Vec<ComputedRepoInfo>,
}

#[handler]
pub async fn repo_list(session: &Session) -> Result<HtmlOrRedirect<String>> {
    let scopes = gather_tc_scopes(session).await?;

    let allowed_repos = scopes
        .iter()
        .filter_map(|scope| scope.strip_prefix("berger:get-repo:"))
        .collect::<HashSet<_>>();

    let mut conn = db::POOL.get().unwrap().acquire().await.unwrap();

    let repos = sqlx::query_as_unchecked!(RepoInfo, "SELECT DISTINCT ON (1, 2, 3) repo_org, repo_name, git_ref, FIRST_VALUE(id) OVER (PARTITION BY (repo_org, repo_name, git_ref) ORDER BY created_at DESC) AS task_id FROM task_groups").fetch_all(&mut conn).await?;
    let allow_all_repos = allowed_repos.contains("*");

    let repos = repos
        .into_iter()
        .filter_map(|mut info| {
            let repo_name = format!("{}/{}", info.repo_org, info.repo_name);
            if !allow_all_repos && !allowed_repos.contains(repo_name.as_str()) {
                return None;
            }

            if !info.git_ref.starts_with("refs/heads") {
                return None;
            }

            info.git_ref = info.git_ref.strip_prefix("refs/heads/").unwrap().into();

            Some(info)
        })
        .group_by(|info| (info.repo_org.clone(), info.repo_name.clone()))
        .into_iter()
        .map(|((repo_org, repo_name), info)| {
            let branches = info.map(|i| (i.git_ref, i.task_id)).collect::<Vec<_>>();
            ComputedRepoInfo {
                repo_org,
                repo_name,
                branches,
            }
        })
        .collect::<Vec<_>>();

    let tpl = RepoListContext {
        base: get_context_for("repo_list", session),
        repos,
    };

    Ok(Html(tpl.render().unwrap()).into())
}

#[derive(Debug, serde::Deserialize)]
pub struct TriggerNewJobParams {
    org: String,
    repo: String,
    r#ref: String,
}

#[handler]
pub async fn trigger_new_job(params: Query<TriggerNewJobParams>, session: &Session) -> Result<()> {
    if GITHUB_TRIGGER_HOOK.is_none() {
        anyhow::bail!("No github hook was configured. Cannot trigger a new job")
    }

    let mut hook_parts = GITHUB_TRIGGER_HOOK.as_ref().unwrap().split('/');
    let hook_group = hook_parts
        .next()
        .ok_or_else(|| anyhow::format_err!("GITHUB_TRIGGER_HOOK is malformed."))?;
    let hook_name = hook_parts
        .next()
        .ok_or_else(|| anyhow::format_err!("GITHUB_TRIGGER_HOOK is malformed."))?;

    let (client, _) = crate::views::utils::get_tc_client_and_scopes(session).await?;
    let hooks = Hooks::new(client)?;

    hooks
        .triggerHook(
            hook_group,
            hook_name,
            &serde_json::json!({
                "repo_name": params.repo,
                "repo_org": params.org,
                "branch": params.r#ref,
            }),
        )
        .await?;

    Ok(())
}
