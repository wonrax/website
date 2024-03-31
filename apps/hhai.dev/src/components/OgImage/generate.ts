import satori from "satori";
import OgImage from "./OgImageGeneratorReact";
import { Resvg } from "@resvg/resvg-js";

interface Props {
  title: string;
  description?: string;
}

const interNormal = await (
  await fetch(
    "https://cdn.jsdelivr.net/npm/@fontsource/inter@5.0.16/files/inter-latin-400-normal.woff",
  )
).arrayBuffer();

const interBold = await (
  await fetch(
    "https://cdn.jsdelivr.net/npm/@fontsource/inter@5.0.16/files/inter-latin-700-normal.woff",
  )
).arrayBuffer();

const interMedium = await (
  await fetch(
    "https://cdn.jsdelivr.net/npm/@fontsource/inter@5.0.16/files/inter-latin-500-normal.woff",
  )
).arrayBuffer();

export default async function getOgImageResponse(
  props: Props,
): Promise<Response> {
  // TODO make a cache out of this for efficient build and bandwith
  const svg = await satori(
    await OgImage({
      title: props.title,
      description: props.description,
    }),
    {
      width: 1200,
      height: 630,
      fonts: [
        {
          name: "Inter",
          data: interNormal,
          weight: 400,
          style: "normal",
        },
        {
          name: "Inter",
          data: interBold,
          weight: 700,
          style: "normal",
        },
        {
          name: "Inter",
          data: interMedium,
          weight: 500,
          style: "normal",
        },
      ],
    },
  );

  const resvg = new Resvg(svg);
  const pngData = resvg.render();
  const pngBuffer = pngData.asPng();

  return new Response(pngBuffer, {
    headers: {
      "Content-Type": "image/png",
    },
  });
}
