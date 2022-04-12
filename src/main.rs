mod db;
mod error;
mod models;
mod pulse;
mod views;

use anyhow::Result;

use poem::{
    endpoint::StaticFilesEndpoint,
    get,
    listener::TcpListener,
    session::{CookieConfig, CookieSession, Session},
    web::{cookie::CookieKey, Redirect},
    EndpointExt, Route, Server,
};

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

pub fn get_context_for(module_name: &str, session: &Session) -> Context {
    let mut context = Context::new();
    context.insert("cur_module", module_name);
    context.insert("base_url", &*BASE_URL);
    context.insert(
        "logged_in",
        &session.get::<Credentials>("credentials").is_some(),
    );

    context
}

fn setup_pulse() {
    tokio::spawn(async {
        loop {
            let _ = pulse::start_pulse_handler().await;
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
}

fn setup_logging() {
    if std::env::var("RUST_LOG").is_err() {
        std::env::set_var("RUST_LOG", "info");
    }
    tracing_subscriber::fmt::init();
}

async fn setup_db() -> Result<()> {
    db::init_pool(&std::env::var("DATABASE_URL").unwrap()).await?;
    sqlx::migrate!("./migrations")
        .run(db::POOL.get().unwrap())
        .await?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    setup_logging();
    setup_db().await?;
    setup_pulse();

    let app = Route::new()
        .at("/", get(crate::views::task_list::root))
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
        })
        .catch_error(|err: crate::error::BergerError| async move {
            match err {
                crate::error::BergerError::AuthenticationError => return Redirect::see_other("/"),
            }
        });

    Server::new(TcpListener::bind("0.0.0.0:3000"))
        .run(app)
        .await?;

    Ok(())
}
