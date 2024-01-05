import {
  createSignal,
  createResource,
  type JSXElement,
  type Accessor,
  createRoot,
  createContext,
  useContext,
  type Setter,
} from "solid-js";
import "./BlogComments.scss";
// import { Context } from "./BlogCommentsContextSolid";
import { Remarkable } from "remarkable";
import { set } from "astro/zod";

type ContextType = {
  isOpen: Accessor<boolean>;
  toggle: () => void;
};

function createCommentSheetContext() {
  const [context, setContext] = createSignal<ContextType>({
    isOpen: () => false,
    toggle: () => {
      console.log("toggle default");
    },
  });
  return { SheetContext: context, SetSheetContext: setContext };
}

export const Context = createRoot(createCommentSheetContext);

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

function CommentComponent({
  comment,
  depth,
}: {
  comment: Comment;
  depth: number;
}) {
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

  const [children, setChildren] = createSignal(comment.children);

  // TODO
  const unshift = (c: Comment) => {
    setChildren((children) => {
      return [c, ...(children || [])];
    });
  };

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
      {isReplying() && (
        <CommentEditor
          parentId={comment.id}
          unshift={unshift}
          setReplying={setIsReplying}
        />
      )}
      <ol class="comment-children">
        {children() &&
          children()!.map((c) => (
            <CommentComponent comment={c} depth={depth + 1} />
          ))}
      </ol>
    </li>
  );
}

type CommentContextType = {
  refetch: () => void;
  slug: string;
  // mutate: Setter<Comment[] | undefined>;
};

const CommentContext = createContext<CommentContextType>();

export function Comments({ slug }: { slug: string }) {
  // TODO do not fetch until the first time the sheet is opened
  // TODO prefetch when user hover over the button
  const [comments, { mutate, refetch }] = createResource<Comment[]>(
    async () => {
      const res = await fetch(
        `http://localhost:3000/public/blog/${slug}/comments?page_offset=0&page_size=2&sort=new`
      );

      return await res.json();
    }
  );

  return (
    <CommentContext.Provider value={{ refetch: refetch, slug }}>
      <div class="comments-container">
        <h3>Comments</h3>
        <CommentEditor
          unshift={(c: Comment) => {
            mutate((comments) => {
              return [c, ...(comments || [])];
            });
          }}
        />
        <ol class="comments">
          {comments.state == "ready" ? (
            <>
              {comments().map((c) => (
                <CommentComponent comment={c} depth={0} />
              ))}
            </>
          ) : (
            "Loading..."
          )}
        </ol>
      </div>
    </CommentContext.Provider>
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
    const oldState = isOpen();
    // TODO change focus to the first focusable element in the sheet for
    // accessibility
    if (!oldState) {
      document.addEventListener("keydown", handleEsc);
      document.body.classList.add("noscroll");
    } else {
      document.removeEventListener("keydown", handleEsc);
      document.body.classList.remove("noscroll");
    }
    setIsOpen(!oldState);
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

type CommentSubmission = {
  author_name: string;
  content: string;
  parent_id?: number;
};

export function CommentEditor(props: {
  parentId?: number;
  unshift: (c: Comment) => void;
  setReplying?: Setter<boolean>;
}) {
  const [loading, setLoading] = createSignal(false);
  const [error, setError] = createSignal<Error>();

  const ctx = useContext(CommentContext);

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
            // author_email: form.email.value,
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
      props.setReplying?.(false);
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
      {error() && (
        <div class="error" style={{ color: "red" }}>
          {error()!.message}
        </div>
      )}
      <button type="submit" disabled={loading()}>
        Submit
      </button>
    </form>
  );
}
