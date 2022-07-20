use crate::BASE_URL;
use anyhow::Result;
use oauth2::basic::BasicClient;
use oauth2::{AuthUrl, ClientId, CsrfToken, RedirectUrl, Scope};
use poem::web::Redirect;
use poem::{handler, session::Session, web::Query};
use reqwest::Url;
use serde::Deserialize;
use taskcluster::Credentials;

lazy_static::lazy_static!(
    pub static ref REDIRECT_URL: String = std::env::var("REDIRECT_URL").unwrap_or_else(|_| {
        let url = Url::parse(&BASE_URL).unwrap();
        format!("https://berger.{}/auth/callback", url.host().unwrap())
    });
);

#[derive(Deserialize)]
pub struct Callback {
    code: String,
}

#[handler]
pub async fn logout(session: &Session) -> Result<Redirect> {
    session.remove("credentials");
    Ok(Redirect::see_other("/"))
}

#[handler]
pub async fn callback(
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
            ("redirect_uri", REDIRECT_URL.clone()),
        ])
        .build()?;
    let res = client.execute(req).await?;
    let body: serde_json::Value = res.json().await?;

    let token = match body["access_token"].as_str() {
        Some(token) => token,
        None => return Ok(Redirect::see_other("/?invalid-auth")),
    };

    if let Ok(creds) = fetch_creds(token).await {
        session.set("credentials", creds);
        Ok(Redirect::see_other("/"))
    } else {
        Ok(Redirect::see_other("/?invalid-creds"))
    }
}

pub async fn fetch_creds(token: &str) -> Result<Credentials> {
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
pub async fn login() -> Result<Redirect> {
    let client = BasicClient::new(
        ClientId::new("berger".to_string()),
        None,
        AuthUrl::new(format!("{}/login/oauth/authorize", &*BASE_URL))?,
        None,
    )
    .set_redirect_uri(RedirectUrl::new(REDIRECT_URL.clone())?);

    let (url, _) = client
        .authorize_url(CsrfToken::new_random)
        .add_scope(Scope::new("berger:*".into()))
        .add_scope(Scope::new("hooks:trigger-hook:*".into()))
        .url();

    Ok(Redirect::see_other(url))
}
