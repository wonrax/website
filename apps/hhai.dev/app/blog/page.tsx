import { Metadata } from "next";
import { allBlogPosts, BlogPost } from "contentlayer/generated";
import Link from "next/link";

export async function generateMetadata(): Promise<Metadata> {
  return {
    metadataBase:
      process.env.NODE_ENV == "production" ? new URL("https://hhai.dev") : null,
    title: "hhai.dev blog",
    description: "hhai.dev blog",
    openGraph: {
      title: "hhai.dev blog",
      description: "hhai.dev blog posts",
      siteName: "hhai.dev",
      images: "/images/thumbnail-og.jpg",
    },
  };
}

export default function Blog() {
  return (
    <div className="mx-auto max-w-[732px] mt-4 h-full">
      <p className="px-4 py-2 font-medium text-lg text-gray-400">2023</p>
      <ul className="flex flex-col transition-all">
        {allBlogPosts.map((post) => (
          <li key={post.slug}>
            <Link href={`/blog/${post.slug}`} prefetch={false}>
              <Article blogPost={post} />
            </Link>
          </li>
        ))}
      </ul>
    </div>
  );
}

const Article = ({ blogPost }: { blogPost: BlogPost }) => {
  return (
    <div className="flex flex-row rounded-lg p-4 justify-between items-start group hover:bg-gray-900 hover:bg-opacity-5">
      <div className="flex flex-col gap-2 items-stretch">
        <h3 className="leading-none font-medium text-gray-800">
          {blogPost.title}
        </h3>
        <p className="text-sm leading-none text-gray-500">
          {blogPost.description}
        </p>
      </div>

      <p className="text-xs font-medium leading-none tracking-wide text-gray-500">
        {new Date(blogPost.published)
          .toLocaleDateString("en-UK", {
            year: "numeric",
            month: "long",
            day: "numeric",
          })
          .toUpperCase()}
      </p>
    </div>
  );
};
