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

lazy_static::lazy_static! {
    pub static ref BASE_URL: String = std::env::var("TASKCLUSTER_ROOT_URL").unwrap();
}

pub struct BaseContext<'a> {
    pub cur_module: &'a str,
    pub base_url: &'a str,
    pub logged_in: bool,
}

pub fn get_context_for<'a>(module_name: &'a str, session: &Session) -> BaseContext<'a> {
    BaseContext {
        cur_module: module_name,
        base_url: &*BASE_URL,
        logged_in: session.get::<Credentials>("credentials").is_some(),
    }
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
                crate::error::BergerError::AuthenticationError => Redirect::see_other("/"),
            }
        });

    Server::new(TcpListener::bind("0.0.0.0:3000"))
        .run(app)
        .await?;

    Ok(())
}
