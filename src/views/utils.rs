use poem::{
    web::{Html, Redirect},
    IntoResponse,
};

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
        return Self::Html(html);
    }
}

impl<T: Send + Into<String>> From<Redirect> for HtmlOrRedirect<T> {
    fn from(redirect: Redirect) -> Self {
        return Self::Redirect(redirect);
    }
}
