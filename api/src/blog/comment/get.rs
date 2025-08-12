use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::NaiveDateTime;
use diesel::prelude::*;
use diesel::sql_types::*;
use diesel_async::RunQueryDsl;
use serde::Deserialize;

use crate::{App, error::AppError, identity::MaybeAuthUser};

use super::CommentTree;

#[derive(Deserialize)]
pub struct Queries {
    page_offset: usize,
    page_size: usize,
    sort: Option<SortType>,
}

#[derive(PartialEq)]
enum SortType {
    Best,
    New,
}

impl<'de> Deserialize<'de> for SortType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        match String::deserialize(deserializer)?.as_str() {
            "best" => Ok(SortType::Best),
            "new" => Ok(SortType::New),
            _ => Err(serde::de::Error::custom("invalid sort type")),
        }
    }
}

#[derive(QueryableByName, Debug)]
struct CommentQueryResult {
    #[diesel(sql_type = Nullable<Integer>)]
    id: Option<i32>,
    #[diesel(sql_type = Nullable<Integer>)]
    identity_id: Option<i32>,
    #[diesel(sql_type = Nullable<Text>)]
    author_name: Option<String>,
    #[diesel(sql_type = Nullable<Text>)]
    content: Option<String>,
    #[diesel(sql_type = Nullable<Integer>)]
    parent_id: Option<i32>,
    #[diesel(sql_type = Nullable<Timestamp>)]
    created_at: Option<NaiveDateTime>,
    #[diesel(sql_type = Nullable<BigInt>)]
    votes: Option<i64>,
    #[diesel(sql_type = Nullable<Integer>)]
    depth: Option<i32>,
}

pub async fn get_comments(
    State(ctx): State<App>,
    Path(slug): Path<String>,
    q: Query<Queries>,
    MaybeAuthUser(auth_user): MaybeAuthUser,
) -> Result<Json<Vec<CommentTree>>, AppError> {
    let sort = q.sort.as_ref().unwrap_or(&SortType::Best);

    let mut conn = ctx.diesel.get().await?;

    // Determine the ORDER BY clause based on sort type
    let order_by_clause = match sort {
        SortType::Best => "ORDER BY votes DESC, comments.created_at",
        SortType::New => "ORDER BY comments.created_at DESC",
    };

    // Single SQL template with dynamic ORDER BY
    let sql = format!(
        "
        ----------------------------------------------------------------
        -- First we get the root comments by sorting based on type
        ----------------------------------------------------------------
        WITH RECURSIVE root_comments AS (
            SELECT
                NULL::integer as parent_id,
                comments.id,
                comments.author_name,
                comments.identity_id,
                comments.content,
                0 depth,
                comments.created_at,
                SUM(CASE WHEN votes.score IS NOT NULL
                    THEN votes.score ELSE 0 END) votes
            FROM blog_comments as comments
            LEFT JOIN blog_comment_votes votes
            ON comments.id = votes.comment_id
            WHERE comments.post_id = (
                SELECT id FROM blog_posts
                WHERE category = 'blog' AND slug = $1
            )
            AND comments.parent_id IS NULL
            GROUP BY
                comments.id,
                comments.author_name,
                comments.identity_id,
                comments.content,
                depth,
                comments.created_at
            {}
            LIMIT $2 OFFSET $3
        ----------------------------------------------------------------
        -- Then we recursively get the children comments of those roots
        ----------------------------------------------------------------
        ), t(
            parent_id,
            id,
            author_name,
            identity_id,
            content,
            depth,
            created_at
        )
        AS (
            (
                SELECT
                    parent_id,
                    id,
                    author_name,
                    identity_id,
                    content,
                    depth,
                    created_at
                FROM root_comments
            )
            UNION ALL
            SELECT
                comments.parent_id,
                comments.id,
                comments.author_name,
                comments.identity_id,
                comments.content,
                t.depth + 1,
                comments.created_at
            FROM t
                JOIN blog_comments as comments
                ON (comments.parent_id = t.id)
        )
        ----------------------------------------------------------------
        -- Finally we get the vote count for each comment because
        -- we can't do it in the recursive query
        ----------------------------------------------------------------
        SELECT
            t.parent_id,
            t.id,
            COALESCE(t.author_name, i.traits->>'name') as author_name,
            t.identity_id,
            t.content,
            t.depth,
            t.created_at,
            SUM(CASE WHEN votes.score IS NOT NULL
                THEN votes.score ELSE 0 END) votes
        FROM t LEFT JOIN blog_comment_votes votes
        ON t.id = votes.comment_id
        LEFT JOIN identities i
        ON t.identity_id IS NOT NULL AND t.identity_id = i.id
        GROUP BY
            t.parent_id,
            t.id,
            COALESCE(t.author_name, i.traits->>'name'),
            t.identity_id,
            t.content,
            t.depth,
            t.created_at;
        ",
        order_by_clause
    );

    let rows = diesel::sql_query(&sql)
        .bind::<Text, _>(&slug)
        .bind::<BigInt, _>(q.page_size as i64)
        .bind::<BigInt, _>(q.page_offset as i64)
        .load::<CommentQueryResult>(&mut conn)
        .await?;

    let final_comments = rows
        .into_iter()
        .filter(|c| {
            c.id.is_some()
                && c.author_name.is_some()
                && c.content.is_some()
                && c.created_at.is_some()
                && c.votes.is_some()
                && c.depth.is_some()
        })
        .map(|c| CommentTree {
            id: c.id.unwrap(),
            author_name: c.author_name.unwrap(),
            content: c.content.unwrap(),
            parent_id: c.parent_id,
            created_at: c.created_at.unwrap(),
            children: None,
            upvote: c.votes.unwrap(),
            depth: c.depth.unwrap() as usize,
            is_comment_owner: match c.identity_id {
                Some(id) => Some(id) == auth_user.as_ref().ok().map(|u| u.id),
                None => false,
            },
            is_blog_author: c.identity_id == Some(ctx.config.owner_identity_id),
        })
        .collect();

    let result = intermediate_tree_sort(final_comments, sort);

    Ok(Json(result))
}

fn intermediate_tree_sort(comments: Vec<CommentTree>, sort: &SortType) -> Vec<CommentTree> {
    // Create a map of parent_id -> children
    let mut parent_children_map: HashMap<Option<i32>, Vec<CommentTree>> = HashMap::new();

    for comment in comments {
        parent_children_map
            .entry(comment.parent_id)
            .or_default()
            .push(comment);
    }

    // Function to recursively build the tree and sort children
    fn build_tree_recursive(
        parent_id: Option<i32>,
        parent_children_map: &mut HashMap<Option<i32>, Vec<CommentTree>>,
        sort: &SortType,
    ) -> Vec<CommentTree> {
        if let Some(mut children) = parent_children_map.remove(&parent_id) {
            // Sort children based on the sort type
            match sort {
                SortType::Best => {
                    children.sort_by(|a, b| {
                        b.upvote
                            .cmp(&a.upvote)
                            .then_with(|| a.created_at.cmp(&b.created_at))
                    });
                }
                SortType::New => {
                    children.sort_by(|a, b| b.created_at.cmp(&a.created_at));
                }
            }

            // Recursively build children for each comment
            for child in &mut children {
                let grandchildren = build_tree_recursive(Some(child.id), parent_children_map, sort);
                if !grandchildren.is_empty() {
                    child.children = Some(grandchildren);
                }
            }

            children
        } else {
            Vec::new()
        }
    }

    build_tree_recursive(None, &mut parent_children_map, sort)
}

#[cfg(test)]
mod test {
    use chrono::NaiveDate;

    use super::*;

    #[test]
    fn test_intermediate_tree_sort_with_no_comments() {
        let comments = vec![];
        let result = intermediate_tree_sort(comments, &SortType::Best);
        assert!(result.is_empty(), "Expected no comments in the tree");
    }

    fn create_mock_comment(
        id: i32,
        parent_id: Option<i32>,
        upvote: i64,
        days_ago: i64,
    ) -> CommentTree {
        CommentTree {
            id,
            author_name: format!("Author {}", id),
            content: format!("Content for comment {}", id),
            parent_id,
            created_at: NaiveDate::from_ymd_opt(2023, 1, 1)
                .unwrap()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                - chrono::Duration::try_days(days_ago).unwrap(),
            children: None,
            upvote,
            depth: 0,
            is_comment_owner: false,
            is_blog_author: false,
        }
    }

    #[test]
    fn test_intermediate_tree_sort_best() {
        let comments = vec![
            CommentTree {
                id: 1,
                author_name: "Author 1".to_string(),
                content: "Root comment".to_string(),
                parent_id: None,
                created_at: NaiveDate::from_ymd_opt(2023, 1, 1)
                    .unwrap()
                    .and_hms_opt(0, 0, 0)
                    .unwrap(),
                children: None,
                upvote: 5,
                depth: 0,
                is_comment_owner: false,
                is_blog_author: false,
            },
            CommentTree {
                id: 2,
                author_name: "Author 2".to_string(),
                content: "Child comment".to_string(),
                parent_id: Some(1),
                created_at: NaiveDate::from_ymd_opt(2023, 1, 1)
                    .unwrap()
                    .and_hms_opt(1, 0, 0)
                    .unwrap(),
                children: None,
                upvote: 10,
                depth: 1,
                is_comment_owner: false,
                is_blog_author: false,
            },
        ];

        let result = intermediate_tree_sort(comments, &SortType::Best);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].id, 1);
        assert!(result[0].children.is_some());
        assert_eq!(result[0].children.as_ref().unwrap().len(), 1);
        assert_eq!(result[0].children.as_ref().unwrap()[0].id, 2);
    }

    #[test]
    fn test_intermediate_tree_sort_new() {
        let older_comment = create_mock_comment(1, None, 5, 5);
        let newer_comment = create_mock_comment(2, None, 3, 2);
        let comments = vec![older_comment, newer_comment];

        let result = intermediate_tree_sort(comments, &SortType::New);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].id, 2, "Newer comment should come first");
        assert_eq!(result[1].id, 1, "Older comment should come second");
    }

    #[test]
    fn test_intermediate_tree_sort_best_by_votes() {
        let low_vote_comment = create_mock_comment(1, None, 3, 2);
        let high_vote_comment = create_mock_comment(2, None, 10, 5);
        let comments = vec![low_vote_comment, high_vote_comment];

        let result = intermediate_tree_sort(comments, &SortType::Best);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].id, 2, "Higher voted comment should come first");
        assert_eq!(result[1].id, 1, "Lower voted comment should come second");
    }
}
