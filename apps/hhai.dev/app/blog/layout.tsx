import Link from "next/link";

export default function BlogLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <>
      <div className="min-h-screen">
        <div className="w-full border-b flex flex-row justify-center py-3 mb-8">
          <div className="flex flex-row items-center gap-10 w-[1024px]">
            <div className="flex flex-row gap-3">
              <img
                height={20}
                width={20}
                src="/favicon.svg"
                alt="hhai.dev logo"
              />
              <Link
                href="/"
                className="text-gray-900 tracking-normal text-xl w-fit"
              >
                Ha Huy Long Hai
              </Link>
            </div>
            <Link href="/blog" className="text-gray-600">
              Blog
            </Link>
            <Link href="/blog" className="text-gray-600">
              Snippets
            </Link>
            <Link href="/blog" className="text-gray-600">
              Links
            </Link>
          </div>
        </div>
        {children}
      </div>
      <footer className="py-12 bg-gray-50">
        <div className="w-full max-w-[900px] mx-auto flex flex-row gap-10">
          <Link href="/" className="text-gray-700 font-bold w-fit">
            hhai.dev
          </Link>
          <div className="flex flex-col gap-2">
            <Link href="/" className="text-gray-700">
              home
            </Link>
            <Link href="/blog" className="text-gray-700">
              blog
            </Link>
          </div>
          <div className="flex flex-col gap-2">
            <Link href="/" className="text-gray-700">
              site map
            </Link>
            <Link href="/blog" className="text-gray-700">
              rss
            </Link>
          </div>
          <div className="flex flex-col gap-2">
            <Link href="/" className="text-gray-700">
              linkedin
            </Link>
            <Link href="/blog" className="text-gray-700">
              twitter
            </Link>
          </div>
        </div>
      </footer>
    </>
  );
}
