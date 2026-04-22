// TODO handle basic form validation client side
import {
  createSignal,
  useContext,
  type Setter,
  type JSXElement,
  onMount,
  onCleanup,
} from "solid-js";
import CommentContext from "./CommentSectionContext";
import { type Comment } from "./CommentSection";
import config from "@/config";
import "./CommentEditor.scss";
import { AppState, SetAppState, checkAuthUser } from "@/state";
import { createFetch, fetchAny } from "@/rpc";
import { z } from "zod/v4";
import OverType, {
  toolbarButtons,
  type OverType as OverTypeInstance,
  type Theme,
} from "overtype";

type FormEvent = Event & {
  target: EventTarget & {
    "author-name"?: HTMLInputElement;
    email?: HTMLInputElement;
    content?: HTMLTextAreaElement;
  };
};

// FIXME: proper type validation
const fetchComment = createFetch(z.custom<Comment>());

const commentOvertypeTheme = {
  name: "wrx",
  colors: {
    bgPrimary: "var(--bg-surface)",
    bgSecondary: "var(--bg-color)",
    text: "var(--text-body-heavy)",
    textPrimary: "var(--text-body-heavy)",
    textSecondary: "var(--text-body-medium)",
    h1: "var(--text-heading)",
    h2: "var(--info-heavy)",
    h3: "var(--text-body-medium)",
    strong: "var(--text-heading)",
    em: "var(--info-medium)",
    del: "var(--text-body-light)",
    link: "var(--text-body-link)",
    code: "var(--text-body-heavy)",
    codeBg: "var(--bg-additive-light)",
    blockquote: "var(--text-body-medium)",
    hr: "var(--border-medium)",
    syntaxMarker: "var(--text-body-light)",
    syntax: "var(--text-body-light)",
    cursor: "var(--accent-color)",
    selection: "rgb(var(--accent-light) / 0.28)",
    listMarker: "var(--info-medium)",
    rawLine: "var(--text-body-medium)",
    border: "var(--border-medium)",
    hoverBg: "var(--bg-additive-light)",
    primary: "var(--accent-color)",
    toolbarBg: "var(--bg-additive-lighter)",
    toolbarBorder: "var(--border-medium)",
    toolbarIcon: "var(--text-body-medium)",
    toolbarHover: "var(--bg-additive-light)",
    toolbarActive: "var(--bg-additive-medium)",
    placeholder: "var(--text-body-light)",
  },
  previewColors: {
    bg: "transparent",
    text: "var(--text-body-heavy)",
    h1: "var(--text-heading)",
    h2: "var(--text-heading)",
    h3: "var(--text-heading)",
    strong: "var(--text-heading)",
    em: "var(--text-body-heavy)",
    link: "var(--text-body-link)",
    code: "var(--text-body-heavy)",
    codeBg: "var(--bg-additive-light)",
    blockquote: "var(--text-body-medium)",
    hr: "var(--border-medium)",
  },
} satisfies Theme;

const commentToolbarButtons = [
  toolbarButtons.bold,
  toolbarButtons.italic,
  toolbarButtons.code,
  toolbarButtons.separator,
  toolbarButtons.link,
  toolbarButtons.quote,
  toolbarButtons.separator,
  toolbarButtons.bulletList,
  toolbarButtons.orderedList,
  toolbarButtons.taskList,
  toolbarButtons.separator,
  toolbarButtons.viewMode,
];

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

  let editor: OverTypeInstance | undefined;

  onMount(() => {
    OverType.setTheme(commentOvertypeTheme);

    [editor] = new OverType(`#${editorId}`, {
      showStats: true,
      toolbar: true,
      toolbarButtons: commentToolbarButtons,
      autoResize: true,
      minHeight: "136px",
      maxHeight: "420px",
      fontFamily: "var(--font-family-sans)",
      fontSize: "var(--font-size-base)",
      lineHeight: "var(--line-height-relaxed)",
      padding: "var(--space-8)",
      mobile: {
        fontSize: "16px",
        lineHeight: "var(--line-height-normal)",
        padding: "var(--space-6)",
      },
      placeholder: props.placeholder ?? "Start writing markdown...",
      value: props.content,
      statsFormatter: ({ words, chars, line, column }) =>
        `<span class="overtype-stat">${words} words</span><span class="overtype-stat">${chars} chars</span><span class="overtype-stat">Ln ${line}, Col ${column}</span>`,
      textareaProps: {
        id: "content",
        name: "content",
        autocomplete: "off",
      },
    });
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
      editor?.setValue(""); // clear the editor after submission
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
      <div class="comment-submission__auth">
        {(props.showAuthInfo ?? true) &&
          (AppState.authUser == null ? (
            <>
              <p class="comment-submission__auth-prompt">
                Either
                <button
                  type="button"
                  class="ui-button ui-button--plain comment-submission__login-button"
                  onClick={(e) => {
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
              <button
                type="button"
                class="ui-button ui-button--plain logout-button"
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
              </button>
            </p>
          ))}
      </div>
      <hr />
      <div id={editorId} class="comment-submission__editor" />
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
              type="button"
              disabled={loading()}
              class="ui-button ui-button--ghost"
            >
              Cancel
            </button>
          )}
          <button
            type="submit"
            class="ui-button ui-button--primary"
            disabled={loading()}
          >
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
        class="ui-input"
        type={props.type || "text"}
        placeholder={props.placeholder}
        id={props.id}
        autocomplete="off"
      />
      {props.description != null && (
        <p class="description">{props.description}</p>
      )}
    </div>
  );
}
