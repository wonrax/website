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
        // Use the imagePaths set from the remark-collect-images so we don't have to duplicate the logic for

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

import http from "http";
import sharp from "sharp";

const imageUrl = "https://example.com/your-image"; // Replace with your image URL

// Create an HTTP request to fetch the image
const request = http.get(imageUrl, (response) => {
  // Initialize a buffer to store the image data
  const imageData = [];

  // Detected image type
  let imageType = null;

  // Buffer to hold the last few bytes from the previous chunk
  let previousBytes = Buffer.from([]);

  response.on("data", (chunk) => {
    if (imageType) {
      // We've already detected the image type, close the connection
      request.end();
      return;
    }

    // Combine the previous bytes with the current chunk
    const combinedBytes = Buffer.concat([previousBytes, chunk]);

    // Attempt to detect the image type based on the combined data
    imageType = detectImageType(combinedBytes);

    // End the request if we reach the start of the scan section (FF DA) for JPEG
    if (imageType === "jpeg" && findFFDA(combinedBytes)) {
      request.end();
    }

    // End the request if we find the "IDAT" chunk for PNG
    if (imageType === "png" && findIDAT(combinedBytes)) {
      request.end();
    }

    // Store the current chunk's last few bytes for the next iteration
    previousBytes = Buffer.from(combinedBytes.slice(-10)); // Adjust as needed
  });

  response.on("end", () => {
    if (imageType) {
      console.log("Detected image type:", imageType);
      // Process the image data based on the detected type here
      const imageBuffer = Buffer.concat(imageData);
      parseImageDimensions(imageBuffer, imageType);
    } else {
      console.error("Image type not detected in the image data.");
    }
  });
});

request.on("error", (error) => {
  console.error("Error requesting image:", error);
});

function detectImageType(buffer) {
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

function findFFDA(buffer) {
  // Check if the scan section (FF DA) is found in the buffer
  for (let i = 0; i < buffer.length - 1; i++) {
    if (buffer[i] === 0xff && buffer[i + 1] === 0xda) {
      return true;
    }
  }
  return false;
}

function findIDAT(buffer) {
  // Check if the "IDAT" chunk is found in the buffer
  const bufferString = buffer.toString("utf-8");
  return bufferString.includes("IDAT");
}
