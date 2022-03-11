mod db;
mod models;
mod pulse;
mod views;

use anyhow::Result;
use models::TaskGroup;

use poem::{
    endpoint::StaticFilesEndpoint,
    get, handler,
    listener::TcpListener,
    session::{CookieConfig, CookieSession, Session},
    web::{cookie::CookieKey, Html},
    EndpointExt, Route, Server,
};

use std::collections::HashSet;

use serde::Serialize;
use taskcluster::Credentials;
use tera::{Context, Tera};

lazy_static::lazy_static! {
    pub static ref BASE_URL: String = std::env::var("TASKCLUSTER_ROOT_URL").unwrap();
    pub static ref TEMPLATES: Tera = {
        let mut tera = match Tera::new("templates/**/*") {
            Ok(t) => t,
            Err(e) => {
                println!("Parsing error(s): {}", e);
                ::std::process::exit(1);
            }
        };
        tera.autoescape_on(vec![".html", ".sql"]);
        tera
    };
}

fn get_context_for(module_name: &str, session: &Session) -> Context {
    let mut context = Context::new();
    context.insert("cur_module", module_name);
    context.insert("base_url", &*BASE_URL);
    context.insert(
        "logged_in",
        &session.get::<Credentials>("credentials").is_some(),
    );

    context
}

#[derive(Serialize)]
struct ComputedTaskGroup<'a> {
    group: &'a TaskGroup,
    status: &'a str,
    start: &'a str,
    end: &'a str,
}

#[handler]
async fn root(req: &poem::Request, session: &Session) -> Result<Html<String>> {
    let mut client = taskcluster::ClientBuilder::new(&**BASE_URL);
    if let Some(creds) = session.get::<Credentials>("credentials") {
        client = client.credentials(creds);
    }
    let tc_auth = taskcluster::Auth::new(client)?;
    let mut scopes = tc_auth.currentScopes().await;
    if scopes.is_err() {
        session.remove("credentials");
        scopes = Ok(tc_auth.currentScopes().await?);
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

    Ok(Html(TEMPLATES.render("index.html", &context)?))
}

async fn run_pulse() {
    loop {
        let _ = pulse::start_pulse_handler().await;
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }

    tracing_subscriber::fmt::init();

    db::init_pool(&std::env::var("DATABASE_URL").unwrap()).await?;
    sqlx::migrate!("./migrations")
        .run(db::POOL.get().unwrap())
        .await?;
    tokio::spawn(run_pulse());

    let app = Route::new()
        .at("/", get(root))
        .at("/auth/callback", get(views::auth::callback))
        .at("/auth/login", get(views::auth::login))
        .at("/auth/logout", get(views::auth::logout))
        .nest(
            "/static",
            StaticFilesEndpoint::new("./static").show_files_listing(),
        )
        .with(CookieSession::new(CookieConfig::private(
            CookieKey::generate(),
        )))
        .inspect_all_err(|err| {
            tracing::error!("{:?}", err);
        });

    Server::new(TcpListener::bind("0.0.0.0:3000"))
        .run(app)
        .await?;

    Ok(())
}
