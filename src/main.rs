mod db;
mod models;
mod pulse;

use anyhow::Result;
use models::TaskGroup;
use poem::{
    endpoint::StaticFilesEndpoint, get, handler, listener::TcpListener, web::Html, EndpointExt,
    Route, Server,
};

use serde::Serialize;
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

fn get_context_for(module_name: &str) -> Context {
    let mut context = Context::new();
    context.insert("cur_module", module_name);
    context.insert("base_url", &*BASE_URL);

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
async fn root(req: &poem::Request) -> Result<Html<String>> {
    let mut context = get_context_for("index");
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
    let groups = groups
        .iter()
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
                    ("", "")
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
        .nest(
            "/static",
            StaticFilesEndpoint::new("./static").show_files_listing(),
        )
        .inspect_all_err(|err| {
            tracing::error!("{:?}", err);
        });

    Server::new(TcpListener::bind("0.0.0.0:3000"))
        .run(app)
        .await?;

    Ok(())
}
