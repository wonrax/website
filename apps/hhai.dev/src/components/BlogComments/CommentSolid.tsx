import { Remarkable } from "remarkable";
import { createSignal, For, type JSXElement } from "solid-js";
import CommentEditor from "./CommentEditorSolid";
import { type Comment } from "./CommentSectionSolid";

// https://gist.github.com/mcraz/11349449
function timeSince(date: Date): string {
  const seconds = Math.floor((new Date().getTime() - date.getTime()) / 1000);

  let interval = Math.floor(seconds / 31536000);

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

export default function CommentComponent(props: {
  comment: Comment;
  depth: number;
}): JSXElement {
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

  // eslint-disable-next-line solid/reactivity
  const [children, setChildren] = createSignal(props.comment.children);

  return (
    <li class="comment">
      <div class="comment-header">
        <div class="comment-author">{props.comment.author_name}</div>
        <div class="comment-date">
          {timeSince(new Date(Date.parse(props.comment.created_at + "Z")))}
        </div>
        {/* <div class="comment-upvote">{props.comment.upvote} upvotes</div> */}
        {/* <div>{comment.id}</div> */}
      </div>
      <div
        class="comment-content"
        // See above for safety concerns
        // eslint-disable-next-line solid/no-innerhtml
        innerHTML={md.render(props.comment.content)}
      />
      <div class="comment-action-row">
        <button onClick={() => setIsReplying(true)}>Reply</button>
        {/* <button>Upvote</button> */}
      </div>
      {isReplying() && (
        <CommentEditor
          parentId={props.comment.id}
          unshift={(c: Comment) => {
            setChildren((children) => {
              return [c, ...(children ?? [])];
            });
          }}
          setReplying={setIsReplying}
          placeholder={`Replying to ${props.comment.author_name}`}
        />
      )}
      {children() != null && (
        <ol class="comment-children">
          {
            <For each={children()}>
              {(c) => <CommentComponent comment={c} depth={props.depth + 1} />}
            </For>
          }
        </ol>
      )}
    </li>
  );
}
