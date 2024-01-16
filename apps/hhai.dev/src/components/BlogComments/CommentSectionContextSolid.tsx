import { createContext } from "solid-js";

interface Context {
  refetch: () => void;
  slug: string;
  // mutate: Setter<Comment[] | undefined>;
}

const CommentContext = createContext<Context>();
export default CommentContext;
