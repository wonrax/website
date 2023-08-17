import Link from "next/link";

export default function BlogLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <>
      <div
        className="min-h-screen"
        style={{
          background:
            "linear-gradient(180deg, rgba(227,250,255,1) 0, rgba(223,255,252,0.4) 70px, rgba(254,255,251,1) 220px, rgba(255,255,255,1) 500px)",
        }}
      >
        <div className="w-full max-w-[700px] mx-auto pt-16 pb-4 flex flex-row gap-2 rounded-lg px-4 md:px-0 col-start-2">
          <Link href="/" className="text-cyan-900 font-bold w-fit">
            hhai.dev
          </Link>
          <h3 className="text-cyan-900 font-medium">/</h3>
          <Link href="/blog" className="text-cyan-900 font-medium">
            blog
          </Link>
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
