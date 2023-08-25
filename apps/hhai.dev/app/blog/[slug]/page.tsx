import { BlogPost, allBlogPosts } from "contentlayer/generated";
import { notFound } from "next/navigation";
import { useMDXComponent } from "next-contentlayer/hooks";
import Link from "next/link";
import "./styles.css";
import BlogImage from "./BlogImage";
import { ComponentType } from "react";
import { ScrollToTopButton } from "./ClientComponents";
import { Metadata } from "next";

import {
  BLOG_LINE_LENGTH,
  BLOG_FEATURE_SM_MAX_LENGTH,
  BLOG_FEATURE_LG_MAX_LENGTH,
} from "./constants";

interface PageProps {
  params: { slug: string };
  searchParams: Record<string, string | string[] | undefined>;
}

export async function generateMetadata(props: PageProps): Promise<Metadata> {
  let slug = props.params.slug;
  const post = allBlogPosts.find((post) => post.slug === slug);
  if (!post) return {};
  return {
    metadataBase:
      process.env.NODE_ENV == "production" ? new URL("https://hhai.dev") : null,
    title: post.title,
    description: post.description,
    openGraph: {
      title: post.title,
      description: post.description,
      siteName: "hhai.dev",
      images: "/images/thumbnail-og.jpg",
    },
  };
}

const RAW_SUFFIX = ".raw";

type Heading = {
  slug: string;
  title: string;
  depth: number;
  children: Heading[];
};

export async function generateStaticParams() {
  const paths: { slug: string }[] = [];
  allBlogPosts.forEach((post) => {
    paths.push({ slug: post.slug }, { slug: post.slug + RAW_SUFFIX });
  });
  return paths;
}

const MDXComponents = {
  maxwidth: ({ children }) => {
    return (
      <div className="w-[700px] max-w-full lg:max-w-[700px] lg:min-w-[700px] z-20">
        {children}
      </div>
    );
  },
};

function limitHeadings(headings: Heading[], maxDepth: number) {
  const result: Heading[] = [];
  for (const h of headings) {
    if (h.depth > maxDepth) continue;
    const children = limitHeadings(h.children, maxDepth); // TODO this could be optimized
    result.push({
      ...h,
      children: children,
    });
  }
  return result;
}

export default function BlogPost(props: PageProps) {
  let rawMode = false;
  let slug = props.params.slug;
  if (props.params.slug.endsWith(RAW_SUFFIX)) {
    slug = props.params.slug.replace(RAW_SUFFIX, "");
    rawMode = true;
  }
  const post = allBlogPosts.find((post) => post.slug === slug);
  if (!post) {
    return notFound();
  }
  const MDXContent = useMDXComponent(post.body.code);
  const headings: Heading[] = limitHeadings(JSON.parse(post.toc), 3);
  if (rawMode) {
    return (
      <Article
        mdxContent={MDXContent}
        raw={post.body.raw}
        rawMode={rawMode}
        slug={post.slug}
      />
    );
  }
  const articleStyles = [
    "max-w-full lg:max-w-[900px] px-4 lg:px-0",
    // "lg:-mt-5", // TODO this is a hack to fix the margin of the first element
    "prose prose-a:break-words",
    "prose-pre:px-0 prose-pre:my-0 prose-pre:bg-transparent prose-pre:text-sm prose-pre:leading-relaxed",
    "prose-code:font-normal",
    "prose-td:whitespace-break-spaces prose-td:break-words prose-table:m-0",
    "prose-td:px-4 prose-td:py-2 prose-th:px-4 prose-th:py-2",
    // "prose-img:rounded-2xl",
    "pb-8 flex flex-col items-center col-start-2 order-1",
    "prose-h2:font-medium prose-h3:font-medium",
  ].join(" ");
  return (
    <div className="flex flex-row gap-8 max-w-full items-center xl:items-start justify-center mt-4">
      {/* <div className="col-start-1 row-span-3 justify-self-end">
        <div className="sticky hidden top-6 mt-3 xl:flex flex-col gap-4 items-center h-[94vh] justify-between">
          <Link href="/blog" className="font-medium">
            ‹ Blog
          </Link>
          <ScrollToTopButton />
        </div>
      </div> */}
      {/* <div className="hidden flex-col gap-1 col-start-1 row-start-1 row-span-2 justify-self-start ml-8 max-w-[256px] 2xl:flex">
        <span className="font-medium text-gray-800">Ha Huy Long Hai</span>
        <span className="text-sm font-medium text-gray-400">
          {new Date(post.published).toLocaleDateString("en-UK", {
            year: "numeric",
            month: "long",
            day: "numeric",
          })}
        </span>
        {post.tags && (
          <div>
            {post.tags.map((tag) => (
              <span className="px-2 py-1 rounded-md bg-gray-100 text-xs font-medium text-gray-500 mr-1 mb-1 inline-block">
                #{tag}
              </span>
            ))}
          </div>
        )}
        <Link
          className="block mt-1 text-blue-700 text-sm font-medium"
          href={`/blog/${post.slug + RAW_SUFFIX}`}
          scroll={false}
        >
          View markdown source
        </Link>
      </div> */}
      <div className="hidden xl:block order-2 self-stretch">
        <div className="sticky top-6 text-sm xl:max-w-[300px] px-6 py-2">
          <p className="text-gray-800 tracking-tight mb-2 text-lg">
            On this page
          </p>
          <TableOfContents headings={headings} />
          <div className="mt-6">
            <ScrollToTopButton />
          </div>
        </div>
      </div>
      <div className={articleStyles}>
        <div
          className="flex flex-col self-start items-start gap-4 w-full max-w-[700px] md:px-0 mx-auto col-start-2"
          // style={{ maxWidth: BLOG_FEATURE_SM_MAX_LENGTH }}
        >
          <ArticleHead post={post} />
          {/* <div className="w-full h-[1px] mt-4 bg-gray-200"></div> */}
        </div>
        <div className="hidden flex-col gap-1 w-full max-w-[700px] 2xl:flex">
          <div className="flex flex-row gap-4 items-baseline">
            <span className="font-medium text-gray-800">Ha Huy Long Hai</span>
            <span className="text-sm font-medium text-gray-400">
              {new Date(post.published).toLocaleDateString("en-UK", {
                year: "numeric",
                month: "long",
                day: "numeric",
              })}
            </span>
          </div>
          {post.tags && (
            <div>
              {post.tags.map((tag) => (
                <span className="px-2 py-1 rounded-md bg-gray-100 text-xs font-medium text-gray-500 mr-1 mb-1 inline-block">
                  #{tag}
                </span>
              ))}
            </div>
          )}
        </div>
        <Article
          mdxContent={MDXContent}
          raw={post.body.raw}
          rawMode={rawMode}
          path={post._raw.sourceFileDir}
        />
      </div>
    </div>
  );
}

const TableOfContents = (props: { headings: Heading[] }) => {
  return (
    <>
      {props.headings.map((heading) => (
        <HeadingElement heading={heading} level={0} />
      ))}
    </>
  );
};

const HeadingElement = (props: { heading: Heading; level: number }) => {
  const fontSizeRem = 1; //- 0.1 * props.level;
  const textStyles = props.level === 0 ? "text-gray-600" : "text-gray-400";
  const listStyles = props.level === 0 ? "mt-1" : "mt-0 pl-3";
  return (
    <ul className={listStyles}>
      <a
        href={`#${props.heading.slug}`}
        style={{ fontSize: `${fontSizeRem}em` }}
        className={`block py-0.5 ${textStyles} hover:text-green-500`}
      >
        {props.heading.title}
      </a>
      {props.heading.children.map((heading) => (
        <li>
          <HeadingElement heading={heading} level={props.level + 1} />
        </li>
      ))}
    </ul>
  );
};

const Article = ({
  mdxContent: MDXContent,
  raw,
  rawMode,
  slug,
  path,
}: {
  mdxContent: ComponentType<any>;
  raw?: string;
  rawMode?: boolean;
  slug?: string;
  path?: string;
}) => {
  const styles = [
    "max-w-full",
    "prose prose-a:break-words",
    "prose-pre:px-0 prose-pre:my-0 prose-pre:bg-transparent",
    "prose-td:whitespace-break-spaces prose-table:m-0",
    "prose-img:rounded-2xl",
  ].join(" ");

  const mdxComponents = {
    ...MDXComponents,
    img: (props) => <BlogImage {...props} path={path} />,
  };

  if (rawMode) {
    return (
      <div>
        <pre className="m-4 text-gray-900 text-sm pb-12">{raw}</pre>
        <div className="fixed bottom-4 right-4 px-2 py-1 bg-white shadow-sm border rounded-md leading-none">
          <Link
            className="text-blue-700 text-sm font-medium"
            href={`/blog/${slug}`}
            scroll={false}
          >
            Back to rendered content
          </Link>
        </div>
      </div>
    );
  }
  return <MDXContent components={mdxComponents} />;
};

const ArticleHead = ({ post }: { post: BlogPost }) => {
  return (
    <div className="w-full">
      <h1 className="font-medium text-4xl mb-4 tracking-tight leading-tight text-gray-800">
        {post.title}
      </h1>
    </div>
  );
};
