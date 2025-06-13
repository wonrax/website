use chrono::NaiveDateTime;
use diesel::prelude::*;
use serde::Serialize;

#[derive(Queryable, Selectable, Insertable, AsChangeset, Debug, Serialize, Clone)]
#[diesel(table_name = crate::schema::counters)]
#[diesel(check_for_backend(diesel::pg::Pg))]
pub struct Counter {
    pub id: i32,
    pub key: String,
    pub name: String,
    pub count: i64,
    pub created_at: NaiveDateTime,
    pub updated_at: NaiveDateTime,
}

#[derive(Insertable, Debug)]
#[diesel(table_name = crate::schema::counters)]
pub struct NewCounter {
    pub key: String,
    pub name: String,
    pub count: i64,
}
