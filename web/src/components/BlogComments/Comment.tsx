import { Remarkable } from "remarkable";
import { createSignal, For, Show, type JSXElement, useContext } from "solid-js";
import { CommentSubmission, CommentEditing } from "./CommentEditor";
import { type Comment } from "./CommentSection";
import config from "@/config";
import { fetchAny } from "@/rpc";
import CommentContext from "./CommentSectionContext";
import { timeSince } from "@/utils/time";

export default function CommentComponent(props: {
  comment: Comment;
  depth: number;
  onDelete: () => void;
}): JSXElement {
  const ctx = useContext(CommentContext);

  if (!ctx?.slug) {
    throw new Error("slug not found");
  }

  const md = new Remarkable({
    html: false,
    xhtmlOut: false,
    breaks: false,
    langPrefix: "language-",
    typographer: false,
    quotes: "“”‘’",
  });

  /* eslint-disable-next-line solid/reactivity --
   * Initial content only, not used for reactivity */
  const [content, setContent] = createSignal(props.comment.content);

  const [isReplying, setIsReplying] = createSignal(false);
  const [isEditing, setIsEditing] = createSignal(false);

  /* eslint-disable-next-line solid/reactivity --
   * Initial content only, not used for reactivity */
  const [children, setChildren] = createSignal(props.comment.children);

  return (
    <li class="comment">
      <header class="ui-meta comment-header">
        <span class="comment-num" aria-hidden="true" />
        <span class="comment-author-name">{props.comment.author_name}</span>
        {(props.comment.is_blog_author ?? false) && (
          <span class="comment-badge">author</span>
        )}
        <span class="comment-spacer" />
        <span class="comment-date">
          {timeSince(new Date(Date.parse(props.comment.created_at + "Z")))}
        </span>
      </header>

      <Show when={!isEditing()}>
        <div
          class="comment-content"
          // Comment content is markdown rendered by remarkable. HTML is
          // disabled in the renderer config above, so the only risk is
          // remarkable bugs — accepted tradeoff for now.
          // eslint-disable-next-line solid/no-innerhtml
          innerHTML={md.render(content())}
        />
      </Show>
      <Show when={isEditing()}>
        <div class="comment-editing">
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
        </div>
      </Show>

      <Show when={!isEditing() && !isReplying()}>
        <div class="ui-meta comment-action-row">
          <button
            type="button"
            class="ui-button ui-button--ghost"
            onClick={() => setIsReplying(true)}
          >
            reply
          </button>
          {(props.comment.is_comment_owner ?? false) && (
            <>
              <button
                type="button"
                class="ui-button ui-button--ghost"
                onClick={() => setIsEditing(true)}
              >
                edit
              </button>
              <button
                type="button"
                class="ui-button ui-button--ghost ui-button--danger"
                onClick={() => {
                  if (
                    confirm("Are you sure you want to delete this comment?")
                  ) {
                    fetchAny(
                      `${config.API_URL}/blog/${ctx.slug}/comments/${props.comment.id}`,
                      {
                        method: "DELETE",
                        credentials: "include",
                      }
                    )
                      /* eslint-disable-next-line solid/reactivity --
                       * Not used for reactivity, onDelete won't change */
                      .then(async (res) => {
                        if (!res.ok) {
                          const err = await res.error();

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
                delete
              </button>
            </>
          )}
        </div>
      </Show>

      {isReplying() && (
        <div class="comment-reply">
          <p class="ui-kicker comment-reply__label">
            ↳ reply to {props.comment.author_name}
          </p>
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
        </div>
      )}

      {children() != null && (
        <ol class="comment-children">
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
        </ol>
      )}
    </li>
  );
}
