import { Remarkable } from "remarkable";
import { createSignal } from "solid-js";
import CommentEditor from "./CommentEditorSolid";
import { type Comment } from "./CommentSectionSolid";

// https://gist.github.com/mcraz/11349449
function timeSince(date: Date) {
  var seconds = Math.floor((new Date().getTime() - date.getTime()) / 1000);

  var interval = Math.floor(seconds / 31536000);

  if (interval > 1) {
    return interval + " years";
  }
  interval = Math.floor(seconds / 2592000);
  if (interval > 1) {
    return interval + " months";
  }
  interval = Math.floor(seconds / 86400);
  if (interval > 1) {
    return interval + " days";
  }
  interval = Math.floor(seconds / 3600);
  if (interval > 1) {
    return interval + " hours";
  }
  interval = Math.floor(seconds / 60);
  if (interval > 1) {
    return interval + " minutes";
  }
  return Math.floor(seconds) + " seconds";
}

export default function CommentComponent({
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
          {timeSince(new Date(Date.parse(comment.created_at + "Z")))}
        </div>
        <div class="comment-upvote">{comment.upvote} upvotes</div>
        {/* <div>{comment.id}</div> */}
      </div>
      <div class="comment-content" innerHTML={md.render(comment.content)} />
      <div class="comment-action-row">
        <button onClick={() => setIsReplying(true)}>Reply</button>
        {/* <button>Upvote</button> */}
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
          placeholder={`Replying to ${comment.author_name}`}
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
