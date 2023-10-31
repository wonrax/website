use std::{cell::RefCell, collections::HashMap, rc::Rc};

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use rand::Rng;

pub fn criterion_benchmark(c: &mut Criterion) {
    let mut group = c.benchmark_group("blog_comments");
    for i in [10, 1000, 10000, 100000].iter() {
        let comments = generate_comments(*i, i / 100);
        group.bench_function(BenchmarkId::new("iterative_recursive", i), |b| {
            b.iter(|| iterative_recursive_sort(comments.clone()))
        });
        group.bench_function(BenchmarkId::new("intermediate_tree", i), |b| {
            b.iter(|| intermediate_tree_sort(comments.clone()))
        });
    }
    group.finish();
}

#[derive(Clone)]
struct Comment {
    id: i32,
    author_name: String,
    content: String,
    parent_id: Option<i32>,
    created_at: chrono::NaiveDateTime,
    upvote: i32,
    depth: i32,
}

#[allow(dead_code)]
#[derive(Clone)]
struct CommentView {
    id: i32,
    author_name: String,
    content: String,
    parent_id: Option<i32>,
    created_at: chrono::NaiveDateTime,
    children: Option<Vec<Rc<RefCell<CommentView>>>>,
    upvote: i32,
    depth: usize,
}

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
            upvote: 0,
            depth: depth as i32,
        };
        comments.push(comment);
    }
    comments
}
fn intermediate_tree_sort(comments: Vec<Comment>) -> Vec<CommentView> {
    let num_comments = comments.len();

    let mut nested = flat_comments_to_tree(comments);
    sort_comments_by_upvote(&mut nested);

    let mut result: Vec<CommentView> = Vec::with_capacity(num_comments);
    for comment in nested {
        depth_first_search(comment.clone(), &mut result);
    }

    // children are not needed anymore
    for comment in &mut result {
        comment.children = None;
    }

    result
}

fn flat_comments_to_tree(comments: Vec<Comment>) -> Vec<Rc<RefCell<CommentView>>> {
    let mut tree = HashMap::<i32, Rc<RefCell<CommentView>>>::new();
    let mut final_comments: Vec<Rc<RefCell<CommentView>>> = vec![];

    for comment in comments {
        let c = Rc::new(RefCell::new(CommentView {
            id: comment.id,
            author_name: comment.author_name,
            content: comment.content,
            parent_id: comment.parent_id,
            created_at: comment.created_at,
            children: None,
            upvote: comment.upvote,
            depth: comment.depth as usize,
        }));

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

        if comment.parent_id.is_none() {
            final_comments.push(c.clone());
        }
    }

    final_comments
}

fn sort_comments_by_upvote(comments: &mut Vec<Rc<RefCell<CommentView>>>) {
    // sort the top level comments
    comments.sort_unstable_by_key(|k| (-k.borrow().upvote, k.borrow().created_at));

    // sort the children recursively
    for comment in comments {
        if let Some(children) = comment.borrow_mut().children.as_mut() {
            sort_comments_by_upvote(children);
        }
    }
}

fn depth_first_search(comment: Rc<RefCell<CommentView>>, mut result: &mut Vec<CommentView>) {
    result.push(comment.borrow_mut().to_owned());
    if let Some(children) = comment.borrow().children.as_ref() {
        for child in children {
            depth_first_search(child.clone(), &mut result);
        }
    }
}

fn iterative_recursive_sort(comments: Vec<Comment>) -> Vec<CommentView> {
    use std::borrow::BorrowMut;
    sort_comments(comments.into_iter().map(Rc::new).collect())
        .into_iter()
        .map(|mut comment| CommentView {
            id: comment.id,
            author_name: comment.borrow_mut().author_name.to_owned(),
            content: comment.borrow_mut().content.to_owned(),
            parent_id: comment.parent_id,
            created_at: comment.created_at,
            children: None,
            upvote: comment.upvote,
            depth: comment.depth as usize,
        })
        .collect()
}

fn sort_comments(comments: Vec<Rc<Comment>>) -> Vec<Rc<Comment>> {
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
    let mut i = 0;
    while i < comments.len() {
        let comment = &comments[i];
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
        i += 1;
    }

    // By now, result only contains the root comments
    result.sort_unstable_by_key(|k| (-k.upvote, k.created_at));

    // Sort the children recursively
    let mut curr = 0;
    for top_level_comment in result.clone() {
        let children = top_level_comments_children
            .remove(&top_level_comment.id)
            .unwrap();
        let children_length = children.len();

        let sorted_children = sort_comments(children);

        // emplace the children back into the result array
        for (k, child_comment) in sorted_children.iter().enumerate() {
            result.insert(k + curr + 1, child_comment.to_owned());
        }

        curr += children_length + 1;
    }

    result
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);
