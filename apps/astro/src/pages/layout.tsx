import { Metadata } from "next";
import NextTopLoader from "nextjs-toploader";
import "./global.css";

export async function generateMetadata(): Promise<Metadata> {
  return {
    icons: ["/favicon.svg"],
  };
}

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en">
      <head>
        <link rel="preconnect" href="https://fonts.googleapis.com" />
        <link
          rel="preconnect"
          href="https://fonts.gstatic.com"
          crossOrigin="anonymous"
        />
        <link
          href="https://fonts.googleapis.com/css2?family=Archivo:wght@400;500;700&display=swap"
          rel="stylesheet"
        ></link>
      </head>
      <body>
        <NextTopLoader />
        <section className="text-center text-gray-700 p-2 text-sm w-full bg-yellow-400 bg-opacity-5">
          ⚠️ Site's under construction. Please expect unfinished business in
          every part of the site. ⚠️
        </section>
        {children}
      </body>
    </html>
  );
}
