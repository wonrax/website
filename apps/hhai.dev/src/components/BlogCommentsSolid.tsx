import {
  createSignal,
  createResource,
  type JSXElement,
  type Accessor,
  createRoot,
} from "solid-js";
import "./BlogComments.scss";
// import { Context } from "./BlogCommentsContextSolid";
import { Remarkable } from "remarkable";

type ContextType = {
  isOpen: Accessor<boolean>;
  toggle: () => void;
};

function createContext() {
  const [context, setContext] = createSignal<ContextType>({
    isOpen: () => false,
    toggle: () => {
      console.log("toggle default");
    },
  });
  return { SheetContext: context, SetSheetContext: setContext };
}

export const Context = createRoot(createContext);

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
  // TODO read more on the docs to identify security issues
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

  const [isReplying, setIsReplying] = createSignal(false);

  return (
    <li class={`comment${depth === 0 ? "" : " not-root-comment"}`}>
      <div class="comment-header">
        <div class="comment-author">{comment.author_name}</div>
        <div class="comment-date">
          {new Date(Date.parse(comment.created_at)).toDateString()}
        </div>
        <div class="comment-upvote">{comment.upvote} upvotes</div>
        <div>{comment.id}</div>
      </div>
      <div class="comment-content" innerHTML={md.render(comment.content)} />
      <div class="comment-action-row">
        <button onClick={() => setIsReplying(true)}>Reply</button>
        <button>Upvote</button>
      </div>
      {isReplying() && <CommentEditor parentId={comment.id} />}
      <ol class="comment-children">
        {comment.children?.map((c) => (
          <Comment comment={c} depth={depth + 1} />
        ))}
      </ol>
    </li>
  );
}

export function Comments({ slug }: { slug: string | undefined }) {
  // TODO do not fetch until the first time the sheet is opened
  // TODO prefetch when user hover over the button
  const [comments] = createResource<Comment[]>(async () => {
    const res = await fetch(
      `http://localhost:3000/public/blog/${slug}/comments?page_offset=0&page_size=10`
    );

    return await res.json();
  });

  return (
    <div class="comments-container">
      <h3>Comments</h3>
      <CommentEditor />
      <ol class="comments">
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

export function Sheet({ children }: { children: JSXElement }) {
  const [isOpen, setIsOpen] = createSignal(true);

  function handleEsc(e: KeyboardEvent) {
    if (e.key === "Escape") {
      toggle();
    }
  }

  function toggle() {
    // TODO change focus to the first focusable element in the sheet for
    // accessibility
    if (!isOpen()) {
      document.addEventListener("keydown", handleEsc);
    }
    if (isOpen()) {
      document.removeEventListener("keydown", handleEsc);
    }
    setIsOpen(!isOpen());
    document.body.classList.toggle("noscroll");
  }

  const { SheetContext, SetSheetContext } = Context;

  SetSheetContext((context) => {
    return {
      isOpen: isOpen,
      toggle: toggle,
    };
  });

  return (
    <div class={`side-sheet${isOpen() ? " open" : ""}`}>
      <div class="sheet-overlay" onClick={toggle}></div>
      <div class="sheet-content">{children}</div>
    </div>
  );
}

export default Sheet;

export function Trigger({
  children,
  ...rest
}: {
  children: JSXElement;
  class?: string;
}) {
  function toggle() {
    const { SheetContext, SetSheetContext } = Context;
    const c = SheetContext();
    c.toggle();
  }

  return (
    <button {...rest} onClick={toggle}>
      {children}
    </button>
  );
}

export function CommentEditor(props: { parentId?: number }) {
  function handleCommentSubmit(e: Event) {
    e.preventDefault();
    const form = e.target as EventTarget & {
      name: HTMLInputElement;
      email: HTMLInputElement;
    };

    const target = e.target as HTMLInputElement;

    const content = target.querySelector("#content") as HTMLDivElement;

    fetch("http://localhost:3000/public/blog/adding-comments/comments", {
      method: "POST",
      body: JSON.stringify({
        id: 0,
        author_name: form.name.value,
        content: content.innerText,
        // author_email: form.email.value,
        parent_id: props.parentId,
        created_at: new Date().toISOString(),
        upvote: 0,
        depth: 0,
      }),
      headers: {
        "Content-Type": "application/json",
      },
    });
  }

  return (
    <form class="comment-submission" onSubmit={handleCommentSubmit}>
      <div class="comment-editor">
        <div
          contentEditable
          class="content"
          id="content"
          role="textbox"
          aria-placeholder="Your comment"
        ></div>
      </div>
      <hr />
      <div class="author-info">
        <input
          class="name"
          id="name"
          autocomplete="false"
          type="text"
          placeholder="Your name"
        />
        <input
          class="email"
          id="email"
          autocomplete="false"
          type="email"
          placeholder="Your email"
        />
      </div>
      <button type="submit">Submit</button>
    </form>
  );
}
