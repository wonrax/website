import { createSignal, createEffect, createResource } from "solid-js";
import "./BlogComments.scss";

type Comment = {
  id: number;
  author_name: string;
  content: string;
  parent_id?: number;
  created_at: string;
  children?: Comment[];
  upvote: number;
  depth: number;
};

function Comment({ comment, depth }: { comment: Comment; depth: number }) {
  return (
    <li className={`comment${depth === 0 ? "" : " not-root-comment"}`}>
      <div className="comment__header">
        <div className="comment__author">{comment.author_name}</div>
        <div className="comment__date">
          {new Date(Date.parse(comment.created_at)).toDateString()}
        </div>
        <div className="comment__upvote">{comment.upvote} upvotes</div>
      </div>
      <div className="comment__content">{comment.content}</div>
      <div className="comment__action-row">
        <button>Reply</button>
        <button>Upvote</button>
      </div>
      <ol className="comment__children">
        {comment.children?.map((c) => (
          <Comment comment={c} depth={depth + 1} />
        ))}
      </ol>
    </li>
  );
}

export default function BlogComments({ slug }: { slug: string | undefined }) {
  const [comments] = createResource<Comment[]>(async () => {
    const res = await fetch(
      `http://localhost:3000/public/blog/${slug}/comments?page_offset=0&page_size=10`
    );

    return await res.json();
  });

  return (
    <div className="comments-container">
      <h3>Comments</h3>
      <form className="comment-submission">
        <input type="text" placeholder="Name" />
        <input type="text" placeholder="Your comment" />
        <button type="submit">Submit</button>
      </form>
      <ol className="comments">
        {comments.state == "ready" ? (
          <>
            {comments().map((c) => (
              <Comment comment={c} depth={0} />
            ))}
          </>
        ) : (
          "Loading..."
        )}
      </ol>
    </div>
  );
}
