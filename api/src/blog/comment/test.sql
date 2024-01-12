WITH RECURSIVE
    root_comments AS (SELECT comments.parent_id,
                             comments.id,
                             comments.author_name,
                             comments.content,
                             ARRAY [comments.id],
                             0,
                             comments.created_at,
                             comments.upvote
                      FROM blog_comments as comments
                               JOIN blog_posts as posts ON (posts.id = comments.post_id)
                      WHERE posts.category = 'blog'
                        AND posts.slug = ?
                        AND comments.parent_id IS NULL
                      ORDER BY comments.upvote DESC, comments.created_at
                      LIMIT ? OFFSET ?),
    t(parent_id, id, author_name, content, root, depth, created_at, upvote) AS ((SELECT *
                                                                                 FROM root_comments)
                                                                                UNION ALL
                                                                                SELECT comments.parent_id,
                                                                                       comments.id,
                                                                                       comments.author_name,
                                                                                       comments.content,
                                                                                       array_append(root, comments.id),
                                                                                       t.depth + 1,
                                                                                       comments.created_at,
                                                                                       comments.upvote
                                                                                FROM t
                                                                                         JOIN blog_comments as comments ON (comments.parent_id = t.id))
SELECT *
FROM t
ORDER BY root;
-- ORDER BY root is important because it will make sure
-- the children are always after their parents. This
-- is only needed for the iterative implementation.
-- TODO remove this line when the current implementation
-- is properly tested.
