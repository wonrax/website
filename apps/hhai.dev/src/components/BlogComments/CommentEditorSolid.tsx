import { createSignal, useContext, type Setter } from "solid-js";
import CommentContext from "./CommentSectionContextSolid";
import { type Comment } from "./CommentSectionSolid";

export default function CommentEditor(props: {
  parentId?: number;
  unshift: (c: Comment) => void;
  setReplying?: Setter<boolean>;
  placeholder?: string;
}) {
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<Error>();

  const ctx = useContext(CommentContext);

  if (!ctx?.slug) {
    throw new Error("slug not found");
  }

  async function handleCommentSubmit(e: Event) {
    e.preventDefault();
    setLoading(true);
    const form = e.target as EventTarget & {
      name: HTMLInputElement;
      email: HTMLInputElement;
    };

    const target = e.target as HTMLInputElement;

    const content = target.querySelector("#content") as HTMLDivElement;

    try {
      const resp = await fetch(
        `http://localhost:3000/public/blog/${ctx?.slug}/comments`,
        {
          method: "POST",
          body: JSON.stringify({
            author_name: form.name.value,
            content: content.innerText,
            author_email: form.email.value || null,
            parent_id: props.parentId,
          }),
          headers: {
            "Content-Type": "application/json",
          },
        }
      );

      if (!resp.ok) {
        const err = await resp.json();
        if (err.msg) {
          throw new Error(err.msg);
        }
        throw new Error("unknown error");
      }

      const comment: Comment = await resp.json();

      props.unshift(comment);

      setLoading(false);

      if (props.setReplying) {
        props.setReplying?.(false);
      } else {
        // reset the form
        form.name.value = "";
        form.email.value = "";
        content.innerText = "";
        setError(undefined);
      }
    } catch (e) {
      if (e instanceof Error) setError(e);
      else setError(new Error(`Unknown error: ${e}`));
      setLoading(false);
      return;
    }
  }

  return (
    <form class="comment-submission" onSubmit={handleCommentSubmit}>
      <div class="comment-editor">
        <div
          contentEditable
          class="content"
          id="content"
          role="textbox"
          aria-placeholder={props.placeholder || "Write a comment..."}
        ></div>
      </div>
      {/* <hr /> */}
      <div class="author-info">
        <Input
          id="name"
          type="text"
          placeholder="Your name"
          description="Required"
        />
        <Input
          id="email"
          type="email"
          placeholder="Your email"
          description="Optional, not displayed"
        />
      </div>
      {error() && <div class="error">{error()!.message}</div>}
      <div class="action-row">
        <div class="markdown-hint">
          {/* TODO check if I have the right to use this logo */}
          <svg
            xmlns="http://www.w3.org/2000/svg"
            height="14"
            viewBox="0 0 208 128"
          >
            <rect
              width="198"
              height="118"
              x="5"
              y="5"
              ry="10"
              stroke="var(--text-body-light)"
              stroke-width="10"
              fill="none"
            />
            <path
              fill="var(--text-body-medium)"
              d="M30 98V30h20l20 25 20-25h20v68H90V59L70 84 50 59v39zm125 0l-30-33h20V30h20v35h20z"
            />
          </svg>
          Markdown supported
        </div>
        <div class="button-row">
          {/* TODO set tab index so that submit goes first */}
          {props.parentId && (
            <button
              onclick={(e) => {
                e.preventDefault();
                props.setReplying?.(false);
              }}
              type="submit"
              disabled={loading()}
            >
              Cancel
            </button>
          )}
          <button type="submit" class="primary" disabled={loading()}>
            Submit
          </button>
        </div>
      </div>
    </form>
  );
}

export function Input(props: {
  type?: string;
  placeholder?: string;
  description?: string;
  id?: string;
}) {
  const { id, description, type = "text", placeholder } = props;
  return (
    <div class="input">
      <input
        type={type}
        placeholder={placeholder}
        id={id}
        autocomplete="false"
      />
      {description && <p class="description">{description}</p>}
    </div>
  );
}
