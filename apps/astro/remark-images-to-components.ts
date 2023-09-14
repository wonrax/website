import type { MarkdownVFile } from "@astrojs/markdown-remark";
import type { Image, Parent } from "mdast";
import type { MdxJsxFlowElement, MdxjsEsm } from "mdast-util-mdx";
import { visit } from "unist-util-visit";
import type { Options as AcornOpts } from "acorn";
import { parse } from "acorn";

export function remarkImageToComponent() {
  return function (tree: any, file: MarkdownVFile) {
    if (!file.data.imagePaths) return;

    const importedImages = new Map<string, string>();

    visit(
      tree,
      "image",
      (node: Image, index: number | null, parent: Parent | null) => {
        // TODO since this is the second pass, we assume all the remain images
        // are remote images. But we should check if the image is remote or local
        // anyway
        const size = getRemoteImageSize(node.url);

        // Build a component that's equivalent to <Image src={importName} alt={node.alt} title={node.title} />
        const componentElement: MdxJsxFlowElement = {
          name: "__AstroImage__",
          type: "mdxJsxFlowElement",
          attributes: [
            {
              name: "src",
              type: "mdxJsxAttribute",
              value: node.url,
            },
            { name: "alt", type: "mdxJsxAttribute", value: node.alt || "" },
            {
              name: "width",
              type: "mdxJsxAttribute",
              value: "1234",
            },
            {
              name: "height",
              type: "mdxJsxAttribute",
              value: "567",
            },
          ],
          children: [],
        };

        if (node.title) {
          componentElement.attributes.push({
            type: "mdxJsxAttribute",
            name: "title",
            value: node.title,
          });
        }

        parent!.children.splice(index!, 1, componentElement);
      }
    );
  };
}

export function jsToTreeNode(
  jsString: string,
  acornOpts: AcornOpts = {
    ecmaVersion: "latest",
    sourceType: "module",
  }
): MdxjsEsm {
  return {
    type: "mdxjsEsm",
    value: "",
    data: {
      estree: {
        body: [],
        ...parse(jsString, acornOpts),
        type: "Program",
        sourceType: "module",
      },
    },
  };
}

import https from "https";
import sharp from "sharp";

function getRemoteImageSize(remoteImageUrl: string) {
  // Create an HTTP request to fetch the image
  console.log("fetching image", remoteImageUrl);
  // Detected image type
  let imageType: "png" | "jpeg" | "webp" | null = null;

  // Initialize a buffer to store the image data
  let imageData = Buffer.from([]);
  const request = https.get(remoteImageUrl, (response) => {
    console.log("response image", response.statusCode, response.headers);

    // Buffer to hold the last few bytes from the previous chunk
    let previousBytes = Buffer.from([]);

    let needToPullMore = true;

    response.on("data", (chunk: Buffer) => {
      if (!needToPullMore) return;

      console.log("chunk", chunk.length, chunk);

      // Combine the previous bytes with the current chunk
      const combinedBytes = Buffer.concat([previousBytes, chunk]);

      imageData = Buffer.concat([imageData, chunk]);

      // Attempt to detect the image type based on the combined data
      imageType = detectImageType(imageData);

      // End the request if we reach the start of the scan section (FF DA) for JPEG
      if (imageType === "jpeg" && findFFDA(combinedBytes)) {
        response.socket.end();
        needToPullMore = false;
      }

      // End the request if we find the "IDAT" chunk for PNG
      if (imageType === "png" && findIDAT(combinedBytes)) {
        response.socket.end();
        needToPullMore = false;
      }

      // Store the current chunk's last few bytes for the next iteration
      previousBytes = Buffer.from(combinedBytes.slice(-8)); // Adjust as needed
    });
  });

  request.on("error", (error) => {
    console.error("Error requesting image:", error);
    console.log("Detected image type:", imageType);
  });

  request.on("close", () => {
    console.log("bytes received", imageData.length);
    if (imageType) {
      console.log("Detected image type:", imageType);
      // Process the image data based on the detected type here
      parseImageDimensions(imageData, imageType);
    } else {
      console.error("Image type not detected in the image data.");
    }
  });

  return {
    width: 1200,
    height: 800,
  };
}

function parseImageDimensions(imageBuffer: Buffer, imageType: string) {
  sharp(imageBuffer)
    .metadata()
    .then((metadata) => {
      console.log("metadata", metadata);
    })
    .catch((error) => {
      console.error("Error parsing image dimensions:", error);
    });
}

function detectImageType(buffer: Buffer) {
  // Implement logic to detect the image type based on the buffer contents
  const firstByte = buffer[0];
  const secondByte = buffer[1];

  if (firstByte === 0xff && secondByte === 0xd8) {
    return "jpeg";
  } else if (firstByte === 0x89 && buffer.toString("utf-8", 1, 4) === "PNG") {
    return "png";
  } else if (
    buffer.toString("utf-8", 0, 4) === "RIFF" &&
    buffer.toString("utf-8", 8, 12) === "WEBP"
  ) {
    return "webp";
  }

  // Add more checks for other image formats as needed

  return null; // If no known image type is detected
}

function findFFDA(buffer: Buffer) {
  // Check if the scan section (FF DA) is found in the buffer
  for (let i = 0; i < buffer.length - 1; i++) {
    if (buffer[i] === 0xff && buffer[i + 1] === 0xda) {
      return true;
    }
  }
  return false;
}

function findIDAT(buffer: Buffer) {
  // Check if the "IDAT" chunk is found in the buffer
  const bufferString = buffer.toString("utf-8");
  return bufferString.includes("IDAT");
}
