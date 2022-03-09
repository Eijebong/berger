mod db;
mod models;
mod pulse;

use anyhow::Result;
use models::TaskGroup;
use oauth2::basic::BasicClient;
use oauth2::{AuthUrl, ClientId, CsrfToken, RedirectUrl, Scope};
use poem::web::Redirect;
use poem::{
    endpoint::StaticFilesEndpoint,
    get, handler,
    listener::TcpListener,
    session::{CookieConfig, CookieSession, Session},
    web::{cookie::CookieKey, Html, Query},
    EndpointExt, Route, Server,
};
use reqwest::Url;
use std::collections::HashSet;

use serde::{Deserialize, Serialize};
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
async fn logout(session: &Session) -> Result<Redirect> {
    session.remove("credentials");
    Ok(Redirect::see_other("/"))
}

#[handler]
async fn login() -> Result<Redirect> {
    let client = BasicClient::new(
        ClientId::new("berger".to_string()),
        None,
        AuthUrl::new(format!("{}/login/oauth/authorize", &*BASE_URL))?,
        None,
    )
    .set_redirect_uri(RedirectUrl::new(
        std::env::var("REDIRECT_URL").unwrap_or_else(|_| {
            let url = Url::parse(&**BASE_URL).unwrap();
            format!("https://berger.{}", url.host().unwrap())
        }),
    )?);

    let (url, _) = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("berger:*".into()))
        .url();

    Ok(Redirect::see_other(url))
}

#[derive(Deserialize)]
struct Callback {
    code: String,
}

#[handler]
async fn callback(
    Query(Callback { code }): Query<Callback>,
    session: &Session,
) -> Result<Redirect> {
    let client = reqwest::Client::new();
    let req = client
        .post(format!("{}/login/oauth/token", &*BASE_URL))
        .form(&[
            ("code", code),
            ("grant_type", "authorization_code".into()),
            ("client_id", "berger".into()),
            ("redirect_uri", "http://127.0.0.1:3000/auth/callback".into()),
        ])
        .build()?;
    let res = client.execute(req).await?;
    let body: serde_json::Value = res.json().await?;
    if let Some(token) = body["access_token"].as_str() {
        if let Ok(creds) = fetch_creds(token).await {
            session.set("credentials", creds);
            Ok(Redirect::see_other("/"))
        } else {
            Ok(Redirect::see_other("/?invalid-creds"))
        }
    } else {
        Ok(Redirect::see_other("/?invalid-auth"))
    }
}

async fn fetch_creds(token: &str) -> Result<Credentials> {
    let client = reqwest::Client::new();
    let req = client
        .get(format!("{}/login/oauth/credentials", &*BASE_URL))
        .header("Authorization", format!("Bearer {}", token))
        .header("Content-Type", "application/json")
        .build()?;
    let res = client.execute(req).await?;
    let creds = &res.json::<serde_json::Value>().await?["credentials"];
    Ok(serde_json::from_value(creds.clone())?)
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
        .at("/auth/callback", get(callback))
        .at("/auth/login", get(login))
        .at("/auth/logout", get(logout))
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
