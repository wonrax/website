import { z } from "zod/v4";

export const ApiError = z.object({
  error: z.optional(z.string()),
  msg: z.string(),
  reason: z.optional(z.string()),
  // debug_info: z.optional(z.string()),
});

interface AugmentedResponse<TData> extends Response {
  JSON: () => Promise<TData>;
  error: () => Promise<z.infer<typeof ApiError>>;
}

export function createFetch<TData>(schema: z.ZodType<TData>) {
  return async (...args: Parameters<typeof fetch>) => {
    const response = await fetch(...args);

    const augmentedResponse = response as AugmentedResponse<TData>;

    augmentedResponse.JSON = async () => schema.parse(await response.json());
    augmentedResponse.error = async () => ApiError.parse(await response.json());

    return augmentedResponse;
  };
}

// Useful if the fetcher only cares about whether the request succeeds and if
// not what the error is.
// TODO don't parse the response body when status OK
export const fetchAny = createFetch(z.any());
