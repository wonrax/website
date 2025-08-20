import generate from "@/components/OgImage/generate";
import type { APIRoute } from "astro";

export const GET: APIRoute = async () => {
  return await generate({
    title: "wrx.sh blog index",
  });
};
