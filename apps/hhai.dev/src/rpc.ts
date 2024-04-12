import { z } from "zod";

export const ApiError = z.object({
  error: z.optional(z.string()),
  msg: z.string(),
  reason: z.optional(z.string()),
  // debug_info: z.optional(z.string()),
});
