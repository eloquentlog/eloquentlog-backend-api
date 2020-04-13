pub mod authentication;
pub mod registration;

use rocket::{Request, State, request};
use rocket::request::FromRequest;
use rocket_slog::SyncLogger;

use crate::config::Config;
use crate::db::DbConn;
use crate::model::token::{BrowserCookieTokenClaims, PersonalAccessTokenClaims};
use crate::model::user::User;
use crate::request::token::TokenType;
use crate::request::token::authentication::AuthenticationToken;

/// User
impl<'a, 'r> FromRequest<'a, 'r> for &'a User {
    type Error = ();

    fn from_request(req: &'a Request<'r>) -> request::Outcome<&'a User, ()> {
        let token_type = req
            .guard::<TokenType>()
            .failure_then(|_| request::Outcome::Forward(()))?;

        let authentication_token = req
            .guard::<AuthenticationToken>()
            .failure_then(|v| request::Outcome::Failure((v.0, ())))?;

        let login = req.local_cache(|| {
            let config = req.guard::<State<Config>>().unwrap();
            let db_conn = req.guard::<DbConn>().unwrap();
            let logger = req.guard::<SyncLogger>().unwrap();

            match token_type {
                TokenType::BrowserCookieToken => {
                    User::find_by_token::<BrowserCookieTokenClaims>(
                        &authentication_token,
                        &config.authentication_token_issuer,
                        &config.authentication_token_secret,
                        &db_conn,
                        &logger,
                    )
                },
                TokenType::PersonalAccessToken => {
                    User::find_by_token::<PersonalAccessTokenClaims>(
                        &authentication_token,
                        &config.authentication_token_issuer,
                        &config.authentication_token_secret,
                        &db_conn,
                        &logger,
                    )
                },
            }
        });
        if let Some(ref user) = login {
            return request::Outcome::Success(user);
        }
        request::Outcome::Forward(())
    }
}
