import {
  createSignal,
  createResource,
  createContext,
  useContext,
  type Setter,
  createEffect,
} from "solid-js";
import "./BlogComments.scss";
import { Remarkable } from "remarkable";
import SheetContext from "./SheetContextSolid";

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

  const [isReplying, setIsReplying] = createSignal(false);

  const [children, setChildren] = createSignal(comment.children);

  return (
    <li class={`comment${depth === 0 ? "" : " not-root-comment"}`}>
      <div class="comment-header">
        <div class="comment-author">{comment.author_name}</div>
        <div class="comment-date">
          {new Date(Date.parse(comment.created_at)).toISOString()}
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
          unshift={(c: Comment) => {
            setChildren((children) => {
              return [c, ...(children || [])];
            });
          }}
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

type Context = {
  refetch: () => void;
  slug: string;
  // mutate: Setter<Comment[] | undefined>;
};

const CommentContext = createContext<Context>();

export function Comments({ slug }: { slug: string }) {
  // check if query string contains open comments on page load
  // if so, open the comments
  // const { SheetContext: sheetCtx } = SheetContext;
  const params = new URLSearchParams(window.location.search);
  const openComments = params.get("open_comments");
  if (openComments) {
    createEffect((success: boolean) => {
      if (!success) {
        if (SheetContext.SheetContext().initialized) {
          SheetContext.SheetContext().toggle();
          return true;
        }
      }
      return false;
    }, false);
  }

  // TODO do not fetch until the first time the sheet is opened
  // TODO prefetch when user hover over the button
  const [comments, { mutate, refetch }] = createResource<Comment[]>(
    async () => {
      const res = await fetch(
        `http://localhost:3000/public/blog/${slug}/comments?page_offset=0&page_size=10&sort=best`
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
