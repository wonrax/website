import {
  createSignal,
  createResource,
  createContext,
  useContext,
  type Setter,
  createEffect,
  lazy,
} from "solid-js";
import SheetContext from "./SheetContextSolid";
import CommentContext from "./CommentsContext";

const CommentComponent = lazy(() => import("./BlogCommentComponentSolid"));
const CommentEditor = lazy(() => import("./BlogCommentEditorSolid"));

export type Comment = {
  id: number;
  author_name: string;
  content: string;
  parent_id?: number;
  created_at: string;
  children?: Comment[];
  upvote: number;
  depth: number;
};

export function Comments() {
  // parse slug from url in format /blog/:slug
  const slug = window.location.pathname.split("/")[2];

  // TODO remove the open_comments query string when the sheet is closed

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

  // hold fetching until the sheet is opened
  const [doFetch, setDoFetch] = createSignal(false);

  const [comments, { mutate, refetch }] = createResource(doFetch, async () => {
    const res = await fetch(
      `http://localhost:3000/public/blog/${slug}/comments?page_offset=0&page_size=10&sort=best`
    );

    return (await res.json()) as Comment[];
  });

  // listen to sheet context
  createEffect(async () => {
    const { SheetContext: sheetCtx } = SheetContext;
    if (
      sheetCtx().initialized &&
      (sheetCtx().isTriggerHover() || sheetCtx().isOpen())
    ) {
      setDoFetch(true);
      CommentComponent.preload();
      CommentEditor.preload();
      import("./BlogComments.scss");
    }
  });

  return (
    <CommentContext.Provider value={{ refetch, slug }}>
      {comments.state == "ready" && comments() ? (
        <div class="comments-container">
          <h3 onclick={() => console.log(useContext(CommentContext))}>
            Comments
          </h3>
          <CommentEditor
            unshift={(c: Comment) => {
              mutate((comments) => {
                return [c, ...(comments || [])];
              });
            }}
          />
          <ol class="comments">
            {comments()!.map((c) => (
              <CommentComponent comment={c} depth={0} />
            ))}
          </ol>
        </div>
      ) : (
        "Loading..."
      )}
    </CommentContext.Provider>
  );
}
