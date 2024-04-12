import { Remarkable } from "remarkable";
import { createSignal, For, Show, type JSXElement } from "solid-js";
import { CommentSubmission, CommentEditing } from "./CommentEditor";
import { type Comment } from "./CommentSection";
import { User } from "lucide-solid";
import config from "@/config";
import { ApiError } from "@/rpc";

// https://gist.github.com/mcraz/11349449
function timeSince(date: Date): string {
  const seconds = Math.floor((new Date().getTime() - date.getTime()) / 1000);

  let interval = Math.floor(seconds / 31536000);

  if (interval > 1) {
    return interval + " years";
  }
  interval = Math.floor(seconds / 2592000);
  if (interval > 1) {
    return interval + " months";
  }
  interval = Math.floor(seconds / 86400);
  if (interval > 1) {
    return interval + " days";
  }
  interval = Math.floor(seconds / 3600);
  if (interval > 1) {
    return interval + " hours";
  }
  interval = Math.floor(seconds / 60);
  if (interval > 1) {
    return interval + " minutes";
  }
  return Math.floor(seconds) + " seconds";
}

export default function CommentComponent(props: {
  comment: Comment;
  depth: number;
  onDelete: () => void;
}): JSXElement {
  // TODO read more on the docs to identify security issues
  // TODO use memo to avoid re-rendering if possible
  const md = new Remarkable({
    html: false, // Enable HTML tags in source
    xhtmlOut: false, // Use '/' to close single tags (<br />)
    breaks: false, // Convert '\n' in paragraphs into <br>
    langPrefix: "language-", // CSS language prefix for fenced blocks

    // Enable some language-neutral replacement + quotes beautification
    typographer: false,

    // Double + single quotes replacement pairs, when typographer enabled,
    // and smartquotes on. Set doubles to '«»' for Russian, '„“' for German.
    quotes: "“”‘’",

    // Highlighter function. Should return escaped HTML,
    // or '' if the source string is not changed
    // highlight: function (/*str, lang*/) {
    //   return "";
    // },
  });

  const [content, setContent] = createSignal(props.comment.content);

  const [isReplying, setIsReplying] = createSignal(false);
  const [isEditing, setIsEditing] = createSignal(false);

  // eslint-disable-next-line solid/reactivity
  const [children, setChildren] = createSignal(props.comment.children);

  return (
    <li class="comment">
      <div class="comment-header">
        <div class="comment-author">
          <div
            style={{
              "background-color": "var(--bg-additive-light)",
              "border-radius": "50%",
              padding: "6px",
              "line-height": "0",
              color: "var(--text-body-heavy)",
            }}
          >
            <User size={16} stroke-width={1.5} />
          </div>
          {props.comment.author_name}
          <div class="comment-date">
            {timeSince(new Date(Date.parse(props.comment.created_at + "Z")))}
          </div>
          {(props.comment.is_blog_author ?? false) && (
            <p
              // TODO use a class instead of inline styles
              style={{
                "font-size": "0.8em",
                color: "var(--info-heavy)",
                "background-color": "var(--info-light)",
                padding: "2px 4px",
                "border-radius": "4px",
                border: "1px solid var(--info-medium)",
                margin: "0",
              }}
            >
              Author
            </p>
          )}
        </div>
        {/* <div class="comment-upvote">{props.comment.upvote} upvotes</div> */}
        {/* <div>{comment.id}</div> */}
      </div>
      <Show when={!isEditing()}>
        <div
          class="comment-content"
          // See above for safety concerns
          // eslint-disable-next-line solid/no-innerhtml
          innerHTML={md.render(content())}
        />
      </Show>
      <Show when={isEditing()}>
        <CommentEditing
          commentId={props.comment.id}
          setEditing={(value, newContent) => {
            setIsEditing(value);
            if (newContent != null) {
              setContent(newContent);
            }
          }}
          content={content()}
        />
      </Show>
      <Show when={!isEditing() && !isReplying()}>
        <div class="comment-action-row">
          <button onClick={() => setIsReplying(true)}>Reply</button>
          {(props.comment.is_comment_owner ?? false) && (
            <>
              <button onClick={() => setIsEditing(true)}>Edit</button>
              <button
                onClick={() => {
                  if (
                    confirm("Are you sure you want to delete this comment?")
                  ) {
                    fetch(
                      `${config.API_URL}/blog/${"TODO"}/comments/${props.comment.id}`,
                      {
                        method: "DELETE",
                        credentials: "include",
                      },
                    )
                      .then(async (res) => {
                        if (res.status !== 200) {
                          const err = ApiError.parse(await res.json());

                          alert("Failed to delete comment: " + err.msg);
                        } else {
                          props.onDelete();
                        }
                      })
                      .catch((e) => {
                        alert("Failed to delete comment: " + e);
                      });
                  }
                }}
              >
                Delete
              </button>
            </>
          )}
          {/* <button>Upvote</button> */}
        </div>
      </Show>
      {isReplying() && (
        <CommentSubmission
          parentId={props.comment.id}
          unshift={(c: Comment) => {
            setChildren((children) => {
              return [c, ...(children ?? [])];
            });
          }}
          setReplying={setIsReplying}
          placeholder={`Replying to ${props.comment.author_name}`}
        />
      )}
      {children() != null && (
        <ol class="comment-children">
          {
            <For each={children()}>
              {(c) => (
                <CommentComponent
                  comment={c}
                  depth={props.depth + 1}
                  onDelete={() => {
                    setChildren((children) => {
                      return children?.filter((child) => child.id !== c.id);
                    });
                  }}
                />
              )}
            </For>
          }
        </ol>
      )}
    </li>
  );
}
