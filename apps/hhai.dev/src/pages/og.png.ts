import fs from "fs/promises";
import satori from "satori";
import sharp from "sharp";
import OgImage from "@/components/OgImage";

export const GET = async function get() {
  // make a cache out of this for efficient build and bandwith
  const interNormal = await fetch(
    "https://cdn.jsdelivr.net/npm/@fontsource/inter@5.0.16/files/inter-latin-400-normal.woff"
  );
  const interBold = await fetch(
    "https://cdn.jsdelivr.net/npm/@fontsource/inter@5.0.16/files/inter-latin-700-normal.woff"
  );
  //   const robotoData = await fs.readFile("https://cdn.jsdelivr.net/npm/@fontsource/inter@5.0.16/files/inter-vietnamese-400-normal.woff2");

  //   const svg = await satori(
  //     {
  //       type: "h1",
  //       props: {
  //         children: "Hello world",
  //         style: {
  //         //   fontWeight: "400"
  //         }
  //       }
  //     },
  //     {
  //       width: 1200,
  //       height: 630,
  //       fonts: [
  //         {
  //           name: "Roboto",
  //           data: await font.arrayBuffer(),
  //           weight: "normal",
  //           style: "normal",
  //         },
  //       ],
  //     }
  //   );

  const svg = await satori(OgImage(), {
    width: 1200,
    height: 630,
    fonts: [
      {
        name: "Inter",
        data: await interNormal.arrayBuffer(),
        weight: 400,
        style: "normal",
      },
      {
        name: "Inter",
        data: await interBold.arrayBuffer(),
        weight: 700,
        style: "normal",
      },
    ],
  });

  const png = await sharp(Buffer.from(svg)).png().toBuffer();

  return new Response(png, {
    headers: {
      "Content-Type": "image/png",
    },
  });
};
