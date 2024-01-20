// TODO handle basic form validation client side
// TODO enable markdown preview through a toggle
// TODO enable basic markdown editing like bold, italic, link, etc.

import {
  createSignal,
  useContext,
  type Setter,
  type JSXElement,
} from "solid-js";
import CommentContext from "./CommentSectionContextSolid";
import { type Comment } from "./CommentSectionSolid";
import config from "@/config";

export default function CommentEditor(props: {
  parentId?: number;
  unshift: (c: Comment) => void;
  setReplying?: Setter<boolean>;
  placeholder?: string;
}): JSXElement {
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<Error>();
  const [auth, setAuth] = createSignal<{
    name: string;
    email: string;
  }>();

  void fetch(`${config.API_URL}/identity/me`, {
    credentials: "include",
  }).then(async (res) => {
    if (res.ok) {
      const body: {
        traits: {
          email: string;
          name: string;
        };
      } = await res.json();

      setAuth({
        name: body.traits.name,
        email: body.traits.email,
      });
    }
  });

  const ctx = useContext(CommentContext);

  if (ctx?.slug === undefined) {
    throw new Error("slug not found");
  }

  async function handleCommentSubmit(e: Event): Promise<void> {
    e.preventDefault();
    setLoading(true);
    const form = e.target as EventTarget & {
      "author-name"?: HTMLInputElement;
      email?: HTMLInputElement;
    };

    const target = e.target as HTMLInputElement;

    const content = target.querySelector("#content");

    if (content == null || !(content instanceof HTMLDivElement)) {
      throw new Error("content not found");
    }

    try {
      const resp = await fetch(
        `${config.API_URL}/public/blog/${ctx?.slug}/comments`,
        {
          method: "POST",
          body: JSON.stringify({
            author_name:
              form["author-name"]?.value != null &&
              form["author-name"].value.length > 0
                ? form["author-name"].value
                : null,
            content: content.innerText,
            author_email:
              form.email?.value != null && form.email.value.length > 0
                ? form.email.value
                : null,
            parent_id: props.parentId,
          }),
          headers: {
            "Content-Type": "application/json",
          },
          credentials: "include",
        },
      );

      if (!resp.ok) {
        if (resp.headers.get("Content-Type")?.includes("application/json")) {
          const err = await resp.json();
          if (err.msg != null && typeof err.msg === "string") {
            throw new Error(err.msg as string);
          }
        }
        throw new Error("Unexpected error: " + (await resp.text()));
      }

      const comment: Comment = await resp.json();

      props.unshift(comment);

      setLoading(false);

      if (props.setReplying != null) {
        props.setReplying?.(false);
      } else {
        // reset the form
        if (form["author-name"] != null) form["author-name"].value = "";
        if (form.email != null) form.email.value = "";
        content.innerText = "";
        setError(undefined);
      }
    } catch (e) {
      if (e instanceof Error) setError(e);
      else setError(new Error(`Unknown error: ${e as any}`));
      setLoading(false);
    }
  }

  return (
    <form
      class="comment-submission"
      onSubmit={(e) => {
        void handleCommentSubmit(e);
      }}
    >
      <div class="comment-editor">
        <div
          contentEditable
          class="content"
          id="content"
          role="textbox"
          aria-placeholder={props.placeholder ?? "Write a comment..."}
        />
      </div>
      {/* <hr /> */}
      {auth() == null ? (
        <>
          <p
            style={{
              "font-size": "13px",
              color: "var(--text-body-medium)",
              margin: "0 16px 12px 16px",
            }}
          >
            <a
              style={{
                color: "var(--text-body-heavy)",
                "font-weight": "var(--font-weight-medium)",
              }}
              href={`${config.API_URL}/identity/login/oidc/github?last_visit=${window.location.href}`}
            >
              Login using GitHub
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
          Posting as <span class="author-name">{auth()?.name}</span>
        </p>
      )}
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
          {props.parentId != null && (
            <button
              onClick={(e) => {
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
