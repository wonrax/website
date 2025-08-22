// TODO handle basic form validation client side
// TODO enable markdown preview through a toggle
// TODO enable basic markdown editing like bold, italic, link, etc.

import {
  createSignal,
  useContext,
  type Setter,
  type JSXElement,
  createEffect,
  onCleanup,
} from "solid-js";
import CommentContext from "./CommentSectionContext";
import { type Comment } from "./CommentSection";
import config from "@/config";
import "./CommentEditor.scss";
import { AppState, SetAppState, checkAuthUser } from "@/state";
import { createFetch, fetchAny } from "@/rpc";
import { z } from "zod/v4";

// @ts-expect-error Overtype has not been typed yet
import { OverType } from "overtype";

type FormEvent = Event & {
  target: EventTarget & {
    "author-name"?: HTMLInputElement;
    email?: HTMLInputElement;
    content?: HTMLTextAreaElement;
  };
};

// FIXME: proper type validation
const fetchComment = createFetch(z.custom<Comment>());

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

    const resp = await fetchComment(
      `${config.API_URL}/blog/${ctx?.slug}/comments`,
      {
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
      }
    );

    if (!resp.ok) {
      const err = await resp.error();
      throw new Error(err.msg);
    }

    const comment = await resp.JSON();
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

    const resp = await fetchComment(
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
      }
    );

    if (!resp.ok) {
      const err = await resp.error();
      throw new Error(err.msg);
    }

    const comment = await resp.JSON();

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
      showAuthInfo={true}
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

  const editorId = "editor-" + Math.random().toString(36).substring(2, 15);

  let editor: OverType;

  createEffect(() => {
    [editor] = new OverType(`#${editorId}`, {
      showStats: true,
      autoResize: true,
      placeholder: props.placeholder ?? "Start writing markdown...",
      value: props.content,
      textareaProps: {
        id: "content",
        name: "content",
        autocomplete: "false",
      },
      theme: window.matchMedia("(prefers-color-scheme: dark)").matches
        ? "cave"
        : "solar",
    });

    // TODO: this does not work because .setTheme has been removed in 1.2.0
    // window
    //   .matchMedia("(prefers-color-scheme: dark)")
    //   .addEventListener("change", function (e) {
    //     editor.setTheme(e.matches ? "cave" : "solar");
    //   });
  });

  onCleanup(() => {
    if (editor != null) {
      editor.destroy();
    }
  });

  if (AppState.authUser === undefined) {
    void checkAuthUser();
  }

  async function handleCommentSubmit(e: FormEvent): Promise<void> {
    try {
      setLoading(true);
      await props.onSubmit(e);
      editor.setValue(""); // clear the editor after submission
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
                  "align-items": "baseline",
                }}
              >
                Either
                <button
                  style={{
                    display: "inline",
                    padding: "4px 4px",
                    color: "var(--info-heavy)",
                    "background-color": "transparent",
                    "text-decoration": "underline",
                  }}
                  onClick={(e) => {
                    // quick workaround in order not to accidentally submit the form
                    // TODO
                    e.preventDefault();
                    const w = window.open(`${config.API_URL}/login/github`);
                    if (w != null)
                      window.onmessage = (e) => {
                        if (e.source !== w) {
                          return;
                        }
                        if (e.data.auth as boolean) void checkAuthUser();
                      };
                  }}
                >
                  login via GitHub
                </button>
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
                  void fetchAny(`${config.API_URL}/logout`, {
                    method: "POST",
                    credentials: "include",
                  }).then(async (response) => {
                    if (response.ok) {
                      SetAppState({ authUser: null });
                    } else {
                      const err = await response.error();
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
      <div id={editorId} />
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
  return (
    <div class="input">
      <input
        type={props.type || "text"}
        placeholder={props.placeholder}
        id={props.id}
        autocomplete="false"
      />
      {props.description != null && (
        <p class="description">{props.description}</p>
      )}
    </div>
  );
}
