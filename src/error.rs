use poem::session::Session;

#[derive(thiserror::Error, Debug)]
pub enum BergerError {
    AuthenticationError,
}

impl std::fmt::Display for BergerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BergerError::AuthenticationError => return f.write_str("Authentication error"),
        }
    }
}

impl BergerError {
    pub fn authentication_error(session: &Session) -> Self {
        session.remove("credentials");
        Self::AuthenticationError
    }
}
