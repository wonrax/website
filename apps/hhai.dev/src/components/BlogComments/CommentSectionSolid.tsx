import SheetContext from "@/components/Sheet/SheetContextSolid";
import {
  createEffect,
  createResource,
  createSignal,
  lazy,
  useContext,
} from "solid-js";
import CommentContext from "./CommentSectionContextSolid";
import("./CommentSection.scss");

const CommentComponent = lazy(() => import("./CommentSolid"));
const CommentEditor = lazy(() => import("./CommentEditorSolid"));

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

export function CommentSection() {
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
      `http://localhost:3000/public/blog/${slug}/comments?page_offset=0&page_size=99&sort=best`
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
    }
  });

  return (
    <CommentContext.Provider value={{ refetch, slug }}>
      <div class="comments-container">
        <div class="heading">
          <h3 onclick={() => refetch()} style={{ cursor: "pointer" }}>
            Comments
          </h3>
          {comments.state != "ready" && <span class="loader"></span>}
        </div>
        {(comments.state == "ready" || comments.state == "refreshing") &&
          comments() && (
            <>
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
            </>
          )}
      </div>
    </CommentContext.Provider>
  );
}
