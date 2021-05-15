use std::fmt;

use chrono::NaiveDateTime;
use diesel::{Associations, Identifiable, Queryable, debug_query, prelude::*};
use diesel::pg::{Pg, PgConnection};

pub use crate::model::token::Claims;
pub use crate::model::user_email_role::*;
pub use crate::model::user_email_identification_state::*;
pub use crate::schema::user_emails;

use crate::logger::Logger;
use crate::model::Activatable;
use crate::model::user::User;
use crate::util::generate_random_hash;

const VERIFICATION_HASH_LENGTH: i32 = 128;
const VERIFICATION_HASH_SOURCE: &[u8] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz01234567890";

/// NewUserEmail
#[derive(Debug)]
pub struct NewUserEmail {
    pub user_id: i64,
    pub email: String,
    pub role: UserEmailRole,
    pub identification_state: UserEmailIdentificationState,
}

impl Default for NewUserEmail {
    fn default() -> Self {
        Self {
            user_id: -1,           // validation error
            email: "".to_string(), // validation error
            role: UserEmailRole::General,
            identification_state: UserEmailIdentificationState::Pending,
        }
    }
}

impl<'a> From<&'a User> for NewUserEmail {
    fn from(user: &'a User) -> Self {
        Self {
            user_id: user.id,
            email: user.email.to_owned(),
            role: UserEmailRole::Primary,

            ..Default::default()
        }
    }
}

/// UserEmail
#[derive(Associations, Debug, Identifiable, Insertable, Queryable)]
#[belongs_to(User)]
#[table_name = "user_emails"]
pub struct UserEmail {
    pub id: i64,
    pub user_id: i64,
    pub email: Option<String>,
    pub role: UserEmailRole,
    pub identification_state: UserEmailIdentificationState,
    pub identification_token: Option<String>,
    pub identification_token_expires_at: Option<NaiveDateTime>,
    pub identification_token_granted_at: Option<NaiveDateTime>,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

impl fmt::Display for UserEmail {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "<UserEmail {role}>", role = &self.role)
    }
}

impl Clone for UserEmail {
    fn clone(&self) -> Self {
        let role = format!("{}", self.role);
        UserEmail {
            role: UserEmailRole::from(role),
            email: self.email.clone(),
            identification_state: self.identification_state.clone(),
            identification_token: self.identification_token.clone(),

            ..*self
        }
    }
}

impl UserEmail {
    pub fn find_by_id(
        id: i64,
        conn: &PgConnection,
        logger: &Logger,
    ) -> Option<Self> {
        if id < 1 {
            return None;
        }

        let q = user_emails::table.filter(user_emails::id.eq(id)).limit(1);

        info!(logger, "{}", debug_query::<Pg, _>(&q).to_string());

        match q.first::<UserEmail>(conn) {
            Ok(v) => Some(v),
            _ => None,
        }
    }

    /// Finds only a non-verified (pending) UserEmail by identification token.
    pub fn find_by_token<T: Claims>(
        token: &str,
        issuer: &str,
        secret: &str,
        conn: &PgConnection,
        logger: &Logger,
    ) -> Option<Self> {
        let value = match T::decode(token, issuer, secret) {
            Ok(claims) => claims.get_subject(),
            Err(e) => {
                error!(logger, "err: {}", e);
                "".to_string()
            },
        };
        if value.is_empty() {
            return None;
        }

        let q = user_emails::table
            .filter(user_emails::identification_token.eq(value))
            .filter(
                user_emails::identification_state
                    .eq(UserEmailIdentificationState::Pending),
            )
            .limit(1);

        info!(logger, "{}", debug_query::<Pg, _>(&q).to_string());

        match q.first::<UserEmail>(conn) {
            Ok(v) => Some(v),
            _ => None,
        }
    }

    pub fn generate_token() -> String {
        generate_random_hash(VERIFICATION_HASH_SOURCE, VERIFICATION_HASH_LENGTH)
    }

    /// Save a new user_email into user_emails.
    ///
    /// # Note
    ///
    /// `identification_state` is assigned always as pending. And following
    /// columns keep still remaining as NULL until granting token later.
    ///
    /// * identification_token
    /// * identification_token_expires_at
    /// * identification_token_granted_at
    pub fn insert(
        user_email: &NewUserEmail,
        conn: &PgConnection,
        logger: &Logger,
    ) -> Option<Self> {
        let q = diesel::insert_into(user_emails::table).values((
            user_emails::user_id.eq(&user_email.user_id),
            Some(user_emails::email.eq(&user_email.email)),
            user_emails::role.eq(UserEmailRole::Primary),
            user_emails::identification_state
                .eq(UserEmailIdentificationState::Pending),
        ));

        info!(logger, "{}", debug_query::<Pg, _>(&q).to_string());

        match q.get_result::<Self>(conn) {
            Err(e) => {
                error!(logger, "err: {}", e);
                None
            },
            Ok(u) => Some(u),
        }
    }

    pub fn grant_token<T: Claims>(
        &self,
        token: &str,
        issuer: &str,
        secret: &str,
        conn: &PgConnection,
        logger: &Logger,
    ) -> Result<String, &'static str> {
        // TODO: should we check duplication?
        let c = T::decode(token, issuer, secret).expect("Invalid value");

        // for identification
        let q = diesel::update(self).set((
            user_emails::identification_state
                .eq(UserEmailIdentificationState::Pending),
            user_emails::identification_token.eq(c.get_subject()),
            user_emails::identification_token_expires_at
                .eq(c.get_expiration_time()),
            user_emails::identification_token_granted_at.eq(c.get_issued_at()),
        ));

        info!(logger, "{}", debug_query::<Pg, _>(&q).to_string());

        match q.get_result::<Self>(conn) {
            Err(e) => {
                error!(logger, "err: {}", e);
                Err("failed to grant an identification token")
            },
            Ok(user_email) => Ok(user_email.identification_token.unwrap()),
        }
    }

    pub fn is_primary(&self) -> bool {
        self.role == UserEmailRole::Primary
    }
}

impl Activatable for UserEmail {
    fn activate(
        &self,
        conn: &PgConnection,
        logger: &Logger,
    ) -> Result<(), &'static str> {
        // TODO: set identification_token to NULL
        let q = diesel::update(self).set((
            user_emails::identification_state
                .eq(UserEmailIdentificationState::Done),
            user_emails::identification_token.eq(""),
        ));

        info!(logger, "{}", debug_query::<Pg, _>(&q).to_string());

        match q.get_result::<Self>(conn) {
            Err(e) => {
                error!(logger, "err: {}", e);
                Err("failed to activate")
            },
            Ok(_) => Ok(()),
        }
    }
}

#[cfg(test)]
pub mod data {
    use super::*;

    use chrono::{Utc, TimeZone};
    use fnv::FnvHashMap;

    use crate::fnvhashmap;
    use crate::model::user::data::USERS;

    type UserEmailFixture = FnvHashMap<&'static str, UserEmail>;

    lazy_static! {
        pub static ref USER_EMAILS: UserEmailFixture = fnvhashmap! {
            "oswald's primary address" => UserEmail {
                id: 1,
                user_id: USERS.get("oswald").unwrap().id,
                email: Some("oswald@example.org".to_string()),
                role: UserEmailRole::Primary,
                identification_state: UserEmailIdentificationState::Done,
                identification_token: None,
                identification_token_expires_at: None,
                identification_token_granted_at: None,
                created_at: Utc.ymd(2019, 7, 7).and_hms(7, 20, 15).naive_utc(),
                updated_at: Utc.ymd(2019, 7, 7).and_hms(7, 20, 15).naive_utc(),
            },
            "weenie's primary address" => UserEmail {
                id: 2,
                user_id: USERS.get("weenie").unwrap().id,
                email: Some("weenie@example.org".to_string()),
                role: UserEmailRole::Primary,
                identification_state: UserEmailIdentificationState::Done,
                identification_token: None,
                identification_token_expires_at: None,
                identification_token_granted_at: None,
                created_at: Utc.ymd(2019, 7, 7).and_hms(7, 20, 15).naive_utc(),
                updated_at: Utc.ymd(2019, 7, 7).and_hms(7, 20, 15).naive_utc(),
            },
            "hennry's primary address" => UserEmail {
                id: 3,
                user_id: 3,
                email: Some("hennry@example.org".to_string()),
                role: UserEmailRole::Primary,
                identification_state: UserEmailIdentificationState::Done,
                identification_token: None,
                identification_token_expires_at: None,
                identification_token_granted_at: None,
                created_at: Utc.ymd(2019, 7, 7).and_hms(7, 20, 15).naive_utc(),
                updated_at: Utc.ymd(2019, 7, 7).and_hms(7, 20, 15).naive_utc(),
            }
        };
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use chrono::{Duration, Utc};

    use crate::model::user::{User, users};
    use crate::model::token::{VerificationClaims, TokenData};

    use crate::model::test::run;
    use crate::model::user::data::USERS;
    use crate::model::user_email::data::USER_EMAILS;

    #[test]
    fn test_new_user_emails_default() {
        let ue = NewUserEmail {
            ..Default::default()
        };

        assert_eq!(ue.user_id, -1);
        assert_eq!(ue.email, "".to_string());
        assert_eq!(ue.role, UserEmailRole::General);
        assert_eq!(
            ue.identification_state,
            UserEmailIdentificationState::Pending,
        );
    }

    #[test]
    fn test_new_user_email_from_user() {
        run(|conn, _, _| {
            let u = USERS.get("weenie").unwrap();
            let user = diesel::insert_into(users::table)
                .values(u)
                .get_result::<User>(conn)
                .unwrap_or_else(|e| panic!("Error at inserting: {}", e));

            let ue = NewUserEmail::from(&user);

            assert_eq!(ue.user_id, user.id);
            assert_eq!(ue.email, user.email);
            assert_eq!(ue.role, UserEmailRole::Primary);
            assert_eq!(
                ue.identification_state,
                UserEmailIdentificationState::Pending
            );
        });
    }

    #[test]
    fn test_user_email_format() {
        let ue = USER_EMAILS.get("hennry's primary address").unwrap();
        assert_eq!(format!("{}", ue), "<UserEmail primary>");

        let mut ue = ue.clone();
        ue.role = UserEmailRole::General;
        assert_eq!(format!("{}", ue), "<UserEmail general>");
    }

    #[test]
    fn test_find_by_id() {
        run(|conn, _, logger| {
            let u = USERS.get("hennry").unwrap();
            let user_id = diesel::insert_into(users::table)
                .values(u)
                .returning(users::id)
                .get_result::<i64>(conn)
                .unwrap_or_else(|e| panic!("Error inserting: {}", e));

            let mut ue =
                USER_EMAILS.get("hennry's primary address").unwrap().clone();
            ue.user_id = user_id;

            let id = diesel::insert_into(user_emails::table)
                .values(&ue)
                .returning(user_emails::id)
                .get_result::<i64>(conn)
                .unwrap_or_else(|e| panic!("Error inserting: {}", e));

            let result = UserEmail::find_by_id(id, conn, logger);
            let user_email = result.unwrap();
            assert_eq!(user_email.id, id);
        })
    }

    #[test]
    fn test_find_by_token_not_found() {
        run(|conn, config, logger| {
            let u = USERS.get("hennry").unwrap();
            let user_id = diesel::insert_into(users::table)
                .values(u)
                .returning(users::id)
                .get_result::<i64>(conn)
                .unwrap_or_else(|e| panic!("Error inserting: {}", e));

            let mut user_email =
                USER_EMAILS.get("hennry's primary address").unwrap().clone();
            user_email.user_id = user_id;

            let user_email = diesel::insert_into(user_emails::table)
                .values(&user_email)
                .get_result::<UserEmail>(conn)
                .unwrap_or_else(|e| panic!("Error inserting: {}", e));

            let now = Utc::now();
            let data = TokenData {
                value: UserEmail::generate_token(),
                granted_at: now.timestamp(),
                expires_at: (now + Duration::hours(1)).timestamp(),
            };
            let token = VerificationClaims::encode(
                data,
                &config.verification_token_issuer,
                &config.verification_token_key_id,
                &config.verification_token_secret,
            );
            let _ = user_email
                .grant_token::<VerificationClaims>(
                    &token,
                    &config.verification_token_issuer,
                    &config.verification_token_secret,
                    conn,
                    logger,
                )
                .expect("failed to grant an identification token");

            // set state as done
            diesel::update(&user_email)
                .set(
                    user_emails::identification_state
                        .eq(UserEmailIdentificationState::Done),
                )
                .execute(conn)
                .unwrap_or_else(|e| panic!("Error updating: {}", e));

            let result = UserEmail::find_by_token::<VerificationClaims>(
                &token,
                &config.verification_token_issuer,
                &config.verification_token_secret,
                conn,
                logger,
            );
            assert!(result.is_none());
        })
    }

    #[test]
    fn test_find_by_token() {
        run(|conn, config, logger| {
            let u = USERS.get("oswald").unwrap();
            let user_id = diesel::insert_into(users::table)
                .values(u)
                .returning(users::id)
                .get_result::<i64>(conn)
                .unwrap_or_else(|e| panic!("Error inserting: {}", e));

            let mut user_email =
                USER_EMAILS.get("oswald's primary address").unwrap().clone();
            user_email.user_id = user_id;

            let id = diesel::insert_into(user_emails::table)
                .values(&user_email)
                .returning(user_emails::id)
                .get_result::<i64>(conn)
                .unwrap_or_else(|e| panic!("Error inserting: {}", e));

            let now = Utc::now();
            let data = TokenData {
                value: UserEmail::generate_token(),
                granted_at: now.timestamp(),
                expires_at: (now + Duration::hours(1)).timestamp(),
            };
            let token = VerificationClaims::encode(
                data,
                &config.verification_token_issuer,
                &config.verification_token_key_id,
                &config.verification_token_secret,
            );
            let _ = user_email
                .grant_token::<VerificationClaims>(
                    &token,
                    &config.verification_token_issuer,
                    &config.verification_token_secret,
                    conn,
                    logger,
                )
                .expect("failed to grant an identification token");

            let result = UserEmail::find_by_token::<VerificationClaims>(
                &token,
                &config.verification_token_issuer,
                &config.verification_token_secret,
                conn,
                logger,
            );
            let user_email = result.unwrap();
            assert_eq!(user_email.id, id);
            assert_eq!(user_email.user_id, user_id);
        })
    }

    #[test]
    #[should_panic]
    fn test_insert_should_panic_on_failure() {
        run(|conn, _, logger| {
            let u = USERS.get("oswald").unwrap();
            let user = diesel::insert_into(users::table)
                .values(u)
                .get_result::<User>(conn)
                .unwrap_or_else(|e| panic!("Error at inserting: {}", e));

            let ue = NewUserEmail::from(&user);
            let result = UserEmail::insert(&ue, conn, logger);
            assert!(result.is_some());

            // abort: duplicate key value violates unique constraint
            let ue = NewUserEmail::from(&user);
            let result = UserEmail::insert(&ue, conn, logger);
            assert!(result.is_none());
        })
    }

    #[test]
    fn test_insert() {
        run(|conn, _, logger| {
            let u = USERS.get("hennry").unwrap();
            let user = diesel::insert_into(users::table)
                .values(u)
                .get_result::<User>(conn)
                .unwrap_or_else(|e| panic!("Error at inserting: {}", e));

            let ue = NewUserEmail::from(&user);
            let result = UserEmail::insert(&ue, conn, logger);
            assert!(result.is_some());

            let user_email = result.unwrap();
            assert!(user_email.id > 0);
            assert_eq!(user_email.email.unwrap(), ue.email);

            let rows_count: i64 = user_emails::table
                .count()
                .first(conn)
                .expect("failed to count rows");
            assert_eq!(1, rows_count);
        })
    }

    #[test]
    fn test_activate() {
        run(|conn, config, logger| {
            let u = USERS.get("oswald").unwrap();
            let user = diesel::insert_into(users::table)
                .values(u)
                .get_result::<User>(conn)
                .unwrap_or_else(|e| panic!("Error at inserting: {}", e));

            let mut ue =
                USER_EMAILS.get("oswald's primary address").unwrap().clone();
            ue.user_id = user.id;

            let user_email = diesel::insert_into(user_emails::table)
                .values(&ue)
                .get_result::<UserEmail>(conn)
                .unwrap_or_else(|e| panic!("Error inserting: {}", e));

            let now = Utc::now();
            let data = TokenData {
                value: UserEmail::generate_token(),
                granted_at: now.timestamp(),
                expires_at: (now + Duration::hours(1)).timestamp(),
            };
            let token = VerificationClaims::encode(
                data,
                &config.verification_token_issuer,
                &config.verification_token_key_id,
                &config.verification_token_secret,
            );
            let _ = user_email
                .grant_token::<VerificationClaims>(
                    &token,
                    &config.verification_token_issuer,
                    &config.verification_token_secret,
                    conn,
                    logger,
                )
                .expect("failed to grant an identification token");

            let user_email = user_emails::table
                .filter(user_emails::id.eq(user_email.id))
                .limit(1)
                .first::<UserEmail>(conn)
                .unwrap();

            let result = user_email.activate(conn, logger);
            assert!(result.is_ok());
        })
    }

    #[test]
    fn test_grant_token() {
        run(|conn, config, logger| {
            let u = USERS.get("hennry").unwrap();
            let user = diesel::insert_into(users::table)
                .values(u)
                .get_result::<User>(conn)
                .unwrap_or_else(|e| panic!("Error at inserting: {}", e));

            let ue = NewUserEmail::from(&user);
            let result = UserEmail::insert(&ue, conn, logger);
            assert!(result.is_some());

            let user_email = result.unwrap();

            let rows_count: i64 = user_emails::table
                .count()
                .first(conn)
                .expect("failed to count rows");
            assert_eq!(1, rows_count);

            let now = Utc::now();
            let data = TokenData {
                value: UserEmail::generate_token(),
                granted_at: now.timestamp(),
                expires_at: (now + Duration::hours(1)).timestamp(),
            };
            let token = VerificationClaims::encode(
                data,
                &config.verification_token_issuer,
                &config.verification_token_key_id,
                &config.verification_token_secret,
            );
            let identification_token = user_email
                .grant_token::<VerificationClaims>(
                    &token,
                    &config.verification_token_issuer,
                    &config.verification_token_secret,
                    conn,
                    logger,
                )
                .expect("failed to grant an identification token");

            let rows_count: i64 = user_emails::table
                .count()
                .first(conn)
                .expect("failed to count rows");
            assert_eq!(1, rows_count);

            let user_email = user_emails::table
                .filter(user_emails::user_id.eq(user.id))
                .limit(1)
                .first::<UserEmail>(conn)
                .unwrap();

            assert_eq!(
                identification_token,
                user_email.identification_token.unwrap()
            );
        });
    }
}
