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
  return (
    <li class={`comment${depth === 0 ? "" : " not-root-comment"}`}>
      <div class="comment-header">
        <div class="comment-author">{comment.author_name}</div>
        <div class="comment-date">
          {new Date(Date.parse(comment.created_at)).toDateString()}
        </div>
        <div class="comment-upvote">{comment.upvote} upvotes</div>
      </div>
      <div class="comment-content" innerHTML={md.render(comment.content)} />
      <div class="comment-action-row">
        <button>Reply</button>
        <button>Upvote</button>
      </div>
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
  const [isOpen, setIsOpen] = createSignal(false);

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

export function CommentEditor() {
  return (
    <form class="comment-submission">
      <textarea class="content" rows={5} placeholder="Your comment" />
      <div class="author-info">
        <input class="name" type="text" placeholder="Your name" />
        <input class="email" type="email" placeholder="(Optional) Your email" />
      </div>
      <button type="submit">Submit</button>
    </form>
  );
}
