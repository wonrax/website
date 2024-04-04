use std::{cell::RefCell, collections::HashMap, rc::Rc};

use axum::{
    extract::{Path, Query, State},
    Json,
};
use chrono::NaiveDateTime;
use serde::Deserialize;

use crate::{error::Error, identity::MaybeAuthUser, APIContext};

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

pub async fn get_comments(
    State(ctx): State<APIContext>,
    Path(slug): Path<String>,
    q: Query<Queries>,
    MaybeAuthUser(auth_user): MaybeAuthUser,
) -> Result<Json<Vec<CommentTree>>, Error> {
    let sort = q.sort.as_ref().unwrap_or(&SortType::Best);

    struct Query {
        id: Option<i32>,
        identity_id: Option<i32>,
        author_name: Option<String>,
        content: Option<String>,
        parent_id: Option<i32>,
        created_at: Option<NaiveDateTime>,
        votes: Option<i64>,
        depth: Option<i32>,
    }

    let rows;
    match sort {
        SortType::Best => {
            // TODO sort by a separate metrics called ranking_score which
            // down-weights the down-votes (e.g. 0.9) so that the comments with
            // equal up and down-votes appear above the comments with no votes.
            let q = sqlx::query_as!(
                Query,
                "
                ----------------------------------------------------------------
                -- First we get the root comments by sorting by upvote and
                -- created_at
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
                    ORDER BY votes DESC, comments.created_at
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
                slug,
                q.page_size as i64,
                q.page_offset as i64,
            )
            .fetch_all(&ctx.pool)
            .await;

            match q {
                Err(e) => return Err(e.into()),
                Ok(_rows) => {
                    rows = _rows;
                }
            };
        }
        SortType::New => {
            let q = sqlx::query_as!(
                Query,
                "
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
                    ORDER BY comments.created_at DESC
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
                slug,
                q.page_size as i64,
                q.page_offset as i64,
            )
            .fetch_all(&ctx.pool)
            .await;

            match q {
                Err(e) => return Err(e.into()),
                Ok(_rows) => rows = _rows,
            }
        }
    }

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
            is_blog_author: c.identity_id == Some(1), // TODO this is hardcoded
        })
        .collect();

    let result = intermediate_tree_sort(final_comments, sort);

    Ok(Json(result))
}

fn intermediate_tree_sort(mut comments: Vec<CommentTree>, sort: &SortType) -> Vec<CommentTree> {
    // This is needed so that the conversion from flat comments to nested
    // comments is O(n) instead of O(n^2)
    comments.sort_unstable_by_key(|k| (k.id));

    let mut nested = flat_comments_to_tree(comments);

    match sort {
        SortType::New => sort_new(&mut nested),
        SortType::Best => {
            sort_best(&mut nested);
        }
    }

    nested
        .into_iter()
        .map(|c| c.borrow_mut().to_owned())
        .collect()
}

fn flat_comments_to_tree(comments: Vec<CommentTree>) -> Vec<Rc<RefCell<CommentTree>>> {
    let mut tree = HashMap::<i32, Rc<RefCell<CommentTree>>>::with_capacity(comments.len());
    let mut final_comments: Vec<Rc<RefCell<CommentTree>>> = vec![];

    for comment in comments {
        let c = Rc::new(RefCell::new(comment));

        tree.insert(c.borrow().id, c.clone());

        if let Some(parent_id) = c.borrow().parent_id {
            // If this is a child comment, add it to its parent's children
            let parent = tree.get(&parent_id);
            if let Some(parent) = parent {
                let mut mut_parent = parent.borrow_mut();
                if let Some(children) = mut_parent.children.as_mut() {
                    children.push(c.clone());
                } else {
                    let children = vec![c.clone()];
                    mut_parent.children = Some(children);
                }
            }
        };

        if c.borrow().parent_id.is_none() {
            final_comments.push(c.clone());
        }
    }

    final_comments
}

fn sort_best(comments: &mut Vec<Rc<RefCell<CommentTree>>>) {
    // sort the top level comments
    comments.sort_unstable_by_key(|k| (-k.borrow().upvote, k.borrow().created_at));

    // sort the children recursively
    for comment in comments {
        if let Some(children) = comment.borrow_mut().children.as_mut() {
            sort_best(children);
        }
    }
}

fn sort_new(comments: &mut Vec<Rc<RefCell<CommentTree>>>) {
    // sort the top level comments
    comments.sort_by(|a, b| {
        b.borrow()
            .created_at
            .partial_cmp(&a.borrow().created_at)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // sort the children recursively
    for comment in comments {
        if let Some(children) = comment.borrow_mut().children.as_mut() {
            sort_best(children);
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use chrono::NaiveDate;

    // Helper function to create a mock CommentTree
    fn create_mock_comment(
        id: i32,
        parent_id: Option<i32>,
        upvote: i64,
        days_ago: i64,
    ) -> Rc<RefCell<CommentTree>> {
        Rc::new(RefCell::new(CommentTree {
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
        }))
    }

    #[test]
    fn test_flat_comments_to_tree_with_no_comments() {
        let comments = vec![];
        let result = flat_comments_to_tree(comments);
        assert!(result.is_empty(), "Expected no comments in the tree");
    }

    #[test]
    fn test_flat_comments_to_tree_with_nested_comments() {
        let comment1 = create_mock_comment(1, None, 10, 5);
        let comment2 = create_mock_comment(2, Some(1), 5, 4); // Child of comment1
        let comments = vec![comment1.clone(), comment2];

        let result = flat_comments_to_tree(
            comments
                .into_iter()
                .map(|c| c.borrow().to_owned().clone())
                .collect(),
        );
        assert_eq!(result.len(), 1, "Expected one root comment");

        let first_child = result[0].borrow();
        let root_children = &first_child.children.as_ref().unwrap();
        assert_eq!(
            root_children.len(),
            1,
            "Expected one child for the root comment"
        );
    }

    #[test]
    fn test_sort_best() {
        let comment1 = create_mock_comment(1, None, 5, 5);
        let comment2 = create_mock_comment(2, None, 10, 4); // Higher votes
        let mut comments = vec![comment1, comment2];

        sort_best(&mut comments);

        assert_eq!(
            comments[0].borrow().id,
            2,
            "Comment with higher votes should come first"
        );
    }

    #[test]
    fn test_sort_new() {
        let comment1 = create_mock_comment(1, None, 5, 5); // Older
        let comment2 = create_mock_comment(2, None, 5, 4); // Newer
        let mut comments = vec![comment1, comment2];

        sort_new(&mut comments);

        assert_eq!(
            comments[0].borrow().id,
            2,
            "Newer comment should come first"
        );
    }

    #[test]
    fn test_intermediate_tree_sort_with_sort_new() {
        let comment1 = create_mock_comment(1, None, 5, 5).borrow().clone(); // Older
        let comment2 = create_mock_comment(2, None, 5, 4).borrow().clone(); // Newer
        let comments = vec![comment1, comment2];

        let sorted_comments = intermediate_tree_sort(comments, &SortType::New);
        assert_eq!(
            sorted_comments[0].id, 2,
            "Newer comment should come first in SortType::New"
        );
    }

    #[test]
    fn test_intermediate_tree_sort_with_sort_best() {
        let comment1 = create_mock_comment(1, None, 5, 5).borrow().clone();
        let comment2 = create_mock_comment(2, None, 10, 4).borrow().clone(); // Higher votes
        let comments = vec![comment1, comment2];

        let sorted_comments = intermediate_tree_sort(comments, &SortType::Best);
        assert_eq!(
            sorted_comments[0].id, 2,
            "Comment with higher votes should come first in SortType::Best"
        );
    }
}
