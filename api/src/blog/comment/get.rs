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
    use std::{cell::RefCell, collections::HashMap, rc::Rc};

    use rand::Rng;

    use super::{Comment, CommentTree};

    // TODO this test is outputing false positive
    #[test]
    fn test_correctness_using_two_implementations() {
        let comments = generate_comments(10000, 5);
        let tree_based_sorted_comments =
            super::intermediate_tree_sort(comments.clone(), &super::SortType::Best);
        let array_based_sorted_comments = iterative_recursive_sort(comments.clone());

        assert_eq!(tree_based_sorted_comments, array_based_sorted_comments);
    }

    // Generate comments randomly
    fn generate_comments(n: usize, max_depth: usize) -> Vec<Comment> {
        let mut comments = vec![];
        for i in 0..n {
            let depth = rand::thread_rng().gen_range(0..max_depth + 1);
            let comment = Comment {
                id: i as i32,
                author_name: "author".to_string(),
                content: "content".to_string(),
                parent_id: None,
                created_at: chrono::offset::Local::now().naive_local(),
                votes: 0,
                depth: depth as i32,
            };
            comments.push(comment);
        }
        comments
    }

    fn depth_first_search(
        comment: &Rc<RefCell<CommentTree>>,
        mut result: &mut Vec<Rc<RefCell<CommentTree>>>,
    ) {
        result.push(comment.clone());
        if let Some(children) = comment.borrow().children.as_ref() {
            for child in children {
                depth_first_search(child, &mut result);
            }
        }
    }

    // This implementation is not used in production but still kept for testing
    // purposes
    fn iterative_recursive_sort(comments: Vec<Comment>) -> Vec<CommentTree> {
        use std::borrow::BorrowMut;
        sort_vec(comments.into_iter().map(Rc::new).collect())
            .into_iter()
            .map(|mut comment| CommentTree {
                id: comment.id,
                author_name: comment.borrow_mut().author_name.to_owned(),
                content: comment.borrow_mut().content.to_owned(),
                parent_id: comment.parent_id,
                created_at: comment.created_at,
                children: None,
                upvote: comment.votes,
                depth: comment.depth as usize,
            })
            .collect()
    }

    fn sort_vec(comments: Vec<Rc<Comment>>) -> Vec<Rc<Comment>> {
        if comments.len() == 0 {
            return vec![];
        }

        let mut result = Vec::with_capacity(comments.len());

        // Map root comment id to its children
        let mut top_level_comments_children: HashMap<i32, Vec<Rc<Comment>>> = HashMap::new();

        // Because the comments are already sorted by id and depth, the first
        // comment's depth is the most shallow depth
        let root_depth = comments[0].depth;
        let mut current_root_comment_id = comments[0].id;

        for comment in comments {
            if comment.depth > root_depth {
                top_level_comments_children
                    .get_mut(&current_root_comment_id)
                    .unwrap()
                    .push(comment.clone());
            } else {
                // Indicating we have reached a new root comment
                current_root_comment_id = comment.id;
                result.push(comment.clone());
                top_level_comments_children.insert(comment.id, vec![]);
            }
        }

        // By now, result only contains the root comments
        result.sort_unstable_by_key(|k| (-k.votes, k.created_at));

        // Sort the children recursively
        let mut curr = 0;
        for top_level_comment in result.clone() {
            let children = top_level_comments_children
                .remove(&top_level_comment.id)
                .unwrap();
            let children_length = children.len();

            let sorted_children = sort_vec(children);

            // emplace the children back into the result array
            result.splice(curr + 1..curr + 1, sorted_children.into_iter());

            curr += children_length + 1;
        }

        result
    }
}
