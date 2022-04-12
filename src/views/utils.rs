use crate::BASE_URL;
use anyhow::Result;
use poem::{
    session::Session,
    web::{Html, Redirect},
    IntoResponse,
};
use taskcluster::Credentials;

pub enum HtmlOrRedirect<T: Send + Into<String>> {
    Html(Html<T>),
    Redirect(Redirect),
}

impl<T: Send + Into<String>> IntoResponse for HtmlOrRedirect<T> {
    fn into_response(self) -> poem::Response {
        match self {
            HtmlOrRedirect::Html(html) => html.into_response(),
            HtmlOrRedirect::Redirect(redirect) => redirect.into_response(),
        }
    }
}

impl<T: Send + Into<String>> From<Html<T>> for HtmlOrRedirect<T> {
    fn from(html: Html<T>) -> Self {
        Self::Html(html)
    }
}

impl<T: Send + Into<String>> From<Redirect> for HtmlOrRedirect<T> {
    fn from(redirect: Redirect) -> Self {
        Self::Redirect(redirect)
    }
}

pub async fn gather_tc_scopes(session: &Session) -> Result<Vec<String>> {
    let mut client = taskcluster::ClientBuilder::new(&**BASE_URL);
    let mut has_credentials = false;
    if let Some(creds) = session.get::<Credentials>("credentials") {
        client = client.credentials(creds);
        has_credentials = true;
    }
    let tc_auth = taskcluster::Auth::new(client)?;
    let scopes = tc_auth.currentScopes().await.map_err(|e| {
        if has_credentials {
            return crate::error::BergerError::authentication_error(session).into();
        }
        e
    })?;

    Ok(scopes["scopes"]
        .as_array()
        .map(|scopes| {
            scopes
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default())
}
