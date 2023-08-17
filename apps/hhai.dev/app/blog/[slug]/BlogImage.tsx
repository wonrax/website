// this is a mess, please refactor

import fs from "fs";
import { parse } from "svg-parser";
import {
  BLOG_LINE_LENGTH,
  BLOG_FEATURE_SM_MAX_LENGTH,
  BLOG_FEATURE_LG_MAX_LENGTH,
} from "./constants";

type FeatureType = "sm" | "lg" | undefined;

type ImageAttributes = {
  src: string;
  width?: number;
  height?: number;
  sizes?: string;
  srcSet?: string;
};

export default function BlogImage({
  src,
  alt,
  "feature-type": featureType,
  path,
}: {
  src: string;
  alt: string;
  "feature-type": FeatureType;
  path: string;
}) {
  let i: ImageAttributes;
  if (src.endsWith(".svg")) {
    src = src.replace(".svg", "");
    let publicPath: string;
    if (src.startsWith("./")) {
      src = src.replace("./", "");
      publicPath = require(`../../../content/${path}/${src}.svg`);
    } else {
      publicPath = require(`../../../content/blog/images/${src}.svg`);
    }
    i = {
      src: publicPath,
    };

    // get width and height of SVG by parsing the generated local file
    // because the loader does not support getting width and height of SVG
    let localPathOfGeneratedImage: string;
    if (process.env.NODE_ENV !== "production") {
      localPathOfGeneratedImage = publicPath.replace("/_next", "./.next");
    } else {
      localPathOfGeneratedImage = `./out${publicPath}`;
    }
    const f = fs.readFileSync(localPathOfGeneratedImage, {
      encoding: "utf-8",
      flag: "r",
    });
    let parsed = parse(f);
    if (parsed) {
      parsed = parsed.children[0];
    }
    const width = parsed?.properties?.width;
    const height = parsed?.properties?.height;
    if (width && height) {
      i.width = parseInt(width);
      i.height = parseInt(height);
    } else {
      console.warn(
        `SVG image \`${src}\` does not have width and height properties`
      );
    }
  } else {
    let sizes: string;
    switch (featureType) {
      case "sm":
        sizes = `(max-width: ${BLOG_FEATURE_SM_MAX_LENGTH}px) 100vw, ${BLOG_FEATURE_SM_MAX_LENGTH}px`;
        break;
      case undefined:
        sizes = `(max-width: ${BLOG_LINE_LENGTH}px) 100vw, ${BLOG_LINE_LENGTH}px`;
        break;
      default:
        sizes = `(max-width: ${BLOG_FEATURE_LG_MAX_LENGTH}px) 100vw, ${BLOG_FEATURE_LG_MAX_LENGTH}px`;
        break;
    }
    if (src.startsWith("./")) {
      src = src.replace("./", "");
      // We have to do this because otherwise the loader will load all file extensions (e.g. mdx)
      if (src.endsWith(".png")) {
        src = src.replace(".png", "");
        i = require(`../../../content/${path}/${src}.png?format=png&resize&sizes[]=700&sizes[]=900&sizes[]=1400&sizes[]=1920&sizes[]=3072&sizes[]=4096`);
      } else if (src.endsWith(".jpg")) {
        src = src.replace(".jpg", "");
        i = require(`../../../content/${path}/${src}.jpg?format=png&resize&sizes[]=700&sizes[]=900&sizes[]=1400&sizes[]=1920&sizes[]=3072&sizes[]=4096`);
      } else {
        throw new Error(`Unsupported image format: ${src}`);
      }
    } else {
      if (src.endsWith(".png")) {
        src = src.replace(".png", "");
        i = require(`../../../content/blog/images/${src}.png?format=png&resize&sizes[]=700&sizes[]=900&sizes[]=1400&sizes[]=1920&sizes[]=3072&sizes[]=4096`);
      } else if (src.endsWith(".jpg")) {
        src = src.replace(".jpg", "");
        i = require(`../../../content/blog/images/${src}.jpg?format=png&resize&sizes[]=700&sizes[]=900&sizes[]=1400&sizes[]=1920&sizes[]=3072&sizes[]=4096`);
      } else {
        console.log("sjd");
        throw new Error(`Unsupported image format: ${src}`);
      }
    }
    i.sizes = sizes;
  }

  return (
    <span className="static group">
      <img
        src={i.src}
        alt={alt}
        sizes={i.sizes}
        srcSet={i.srcSet}
        width={i.width}
        height={i.height}
        loading="lazy"
        style={{ marginLeft: "auto", marginRight: "auto" }}
      />
      <a
        className="invisible block relative shadow-sm border group-hover:visible left-5 bottom-20 text-gray-700 w-8 h-8 font-medium text-base px-2 py-1 rounded-lg bg-white cursor-pointer no-underline"
        href={i.src}
        target="_blank"
      >
        ↓
      </a>
    </span>
  );
}
