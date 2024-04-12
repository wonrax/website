// TODO handle basic form validation client side
// TODO enable markdown preview through a toggle
// TODO enable basic markdown editing like bold, italic, link, etc.

import {
  createSignal,
  useContext,
  type Setter,
  type JSXElement,
} from "solid-js";
import CommentContext from "./CommentSectionContext";
import { type Comment } from "./CommentSection";
import config from "@/config";
import "./CommentEditor.scss";
import { AppState, SetAppState, checkAuthUser } from "@/state";
import { ApiError } from "@/rpc";

type FormEvent = Event & {
  target: EventTarget & {
    "author-name"?: HTMLInputElement;
    email?: HTMLInputElement;
    content?: HTMLTextAreaElement;
  };
};

export function CommentSubmission(props: {
  parentId?: number;
  unshift: (c: Comment) => void;
  setReplying?: Setter<boolean>;
  placeholder?: string;
}): JSXElement {
  const ctx = useContext(CommentContext);

  if (ctx?.slug === undefined) {
    throw new Error("slug not found");
  }

  async function handleCommentSubmit(e: FormEvent): Promise<void> {
    e.preventDefault();

    const form = e.target;
    if (form.content?.value == null) {
      throw new Error("content is required");
    }

    const resp = await fetch(`${config.API_URL}/blog/${ctx?.slug}/comments`, {
      method: "POST",
      body: JSON.stringify({
        author_name: form["author-name"]?.value,
        content: form.content.value,
        author_email:
          form.email?.value != null && form.email.value === ""
            ? null
            : form.email?.value,
        parent_id: props.parentId,
      }),
      headers: {
        "Content-Type": "application/json",
      },
      credentials: "include",
    });

    if (!resp.ok) {
      const err = ApiError.parse(await resp.json());
      if (err.msg != null && typeof err.msg === "string")
        throw new Error(err.msg);
    }

    const comment: Comment = await resp.json();
    comment.is_comment_owner = true;

    props.unshift(comment);

    if (props.setReplying != null) {
      props.setReplying?.(false);
    } else {
      // reset the form
      if (form["author-name"] != null) form["author-name"].value = "";
      if (form.email != null) form.email.value = "";
      form.content.value = "";
    }
  }

  return (
    <CommentEditorBase
      cancellable={props.parentId != null}
      onSubmit={handleCommentSubmit}
      onCancel={() => {
        if (props.setReplying != null) {
          props.setReplying(false);
        }
      }}
      placeholder={props.placeholder}
    />
  );
}

export function CommentEditing(props: {
  commentId: number;
  content: string;
  setEditing?: (value: boolean, newContent: string | null) => void;
}): JSXElement {
  const ctx = useContext(CommentContext);

  if (ctx?.slug === undefined) {
    throw new Error("slug not found");
  }

  async function handleCommentSubmit(e: FormEvent): Promise<void> {
    e.preventDefault();

    const form = e.target;
    if (form.content?.value == null) {
      throw new Error("content is required");
    }

    const resp = await fetch(
      `${config.API_URL}/blog/${ctx?.slug}/comments/${props.commentId}`,
      {
        method: "PATCH",
        body: JSON.stringify({
          content: form.content.value,
        }),
        headers: {
          "Content-Type": "application/json",
        },
        credentials: "include",
      },
    );

    if (!resp.ok) {
      const err = ApiError.parse(await resp.json());
      throw new Error(err.msg);
    }

    const comment: Comment = await resp.json();

    if (props.setEditing != null) props.setEditing(false, comment.content);
  }

  return (
    <CommentEditorBase
      cancellable={true}
      onSubmit={handleCommentSubmit}
      onCancel={() => {
        if (props.setEditing != null) {
          props.setEditing(false, null);
        }
      }}
      content={props.content}
      showAuthInfo={false}
      buttonCta="Save"
    />
  );
}

export function CommentEditorBase(props: {
  cancellable: boolean;
  onSubmit: (e: FormEvent) => Promise<void>;
  onCancel: () => void;
  placeholder?: string;
  content?: string;
  showAuthInfo?: boolean;
  buttonCta?: string;
}): JSXElement {
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<Error>();

  if (AppState.authUser === undefined) {
    void checkAuthUser();
  }

  async function handleCommentSubmit(e: FormEvent): Promise<void> {
    try {
      setLoading(true);
      await props.onSubmit(e);
    } catch (e) {
      setError(e as Error);
    }

    setLoading(false);
  }

  return (
    <form
      class="comment-submission"
      onSubmit={(e) => {
        void handleCommentSubmit(e);
      }}
    >
      <div style={{ padding: "10px" }}>
        {(props.showAuthInfo ?? true) &&
          (AppState.authUser == null ? (
            <>
              <p
                style={{
                  "font-size": "14px",
                  color: "var(--text-body-medium)",
                  margin: "2px 8px 12px 8px",
                  "line-height": "140%",
                }}
              >
                Either{" "}
                <a
                  style={{
                    color: "var(--text-body-heavy)",
                    "font-weight": "var(--font-weight-medium)",
                    "text-decoration": "underline",
                  }}
                  href={`${config.API_URL}/login/github?return_to=${window.location.href}`}
                >
                  login via GitHub
                </a>{" "}
                or type your name below
              </p>
              <div class="author-info">
                <Input
                  id="author-name"
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
            </>
          ) : (
            <p class="auth-user">
              Posting as{" "}
              <span class="author-name">{AppState.authUser?.name}</span>, or{" "}
              <span
                class="logout-button"
                onClick={() => {
                  void fetch(`${config.API_URL}/logout`, {
                    method: "POST",
                    credentials: "include",
                  }).then(async (response) => {
                    if (response.ok) {
                      SetAppState({ authUser: null });
                    } else {
                      const err = ApiError.parse(await response.json());
                      alert("Failed to log you out: " + err.msg);
                    }
                  });
                }}
              >
                logout
              </span>
            </p>
          ))}
      </div>
      <hr />
      <div class="comment-editor">
        <textarea
          class="content"
          id="content"
          placeholder={props.placeholder ?? "Write a comment..."}
          onKeyUp={(e) => {
            if (e.currentTarget.scrollHeight > e.currentTarget.clientHeight)
              e.currentTarget.style.height =
                e.currentTarget.scrollHeight + "px";
          }}
          value={props.content ?? ""}
        />
      </div>
      {error() != null && <div class="error">{error()?.message}</div>}
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
          {props.cancellable && (
            <button
              onClick={(e) => {
                e.preventDefault();
                props.onCancel();
              }}
              type="submit"
              disabled={loading()}
              class="ghost"
            >
              Cancel
            </button>
          )}
          <button type="submit" class="primary" disabled={loading()}>
            {props.buttonCta ?? "Submit"}
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
}): JSXElement {
  const { id, description, type = "text", placeholder } = props;
  return (
    <div class="input">
      <input
        type={type}
        placeholder={placeholder}
        id={id}
        autocomplete="false"
      />
      {description != null && <p class="description">{description}</p>}
    </div>
  );
}
