import { Metadata } from "next";
import Link from "next/link";

export async function generateMetadata(): Promise<Metadata> {
  return {
    metadataBase:
      process.env.NODE_ENV == "production" ? new URL("https://hhai.dev") : null,
    title: "hhai.dev",
    description: "hhai.dev personal website",
    openGraph: {
      title: "hhai.dev",
      description: "hhai.dev personal website",
      siteName: "hhai.dev",
      images: "/images/thumbnail-og.jpg",
    },
  };
}

export default function Page() {
  const mainColorStyles = "text-gray-900";
  const secondaryColorStyles = "text-gray-400";
  const headingStyles = "text-xl tracking-tighter leading-none";
  const paragraphStyles = "text-base tracking-tight leading-7";
  const linkStyles = `${paragraphStyles} ${mainColorStyles} underline`;
  return (
    <div className="flex flex-col items-center w-full py-16">
      <div className="flex flex-row gap-14">
        <div className="flex flex-col max-w-[295px]">
          <p className={`${headingStyles} ${mainColorStyles} font-medium`}>
            Ha Huy Long Hai
          </p>
          <p className={`${headingStyles} ${secondaryColorStyles} mt-1`}>
            Software Engineer
          </p>
          <p
            className={`${headingStyles} ${mainColorStyles} mt-6 leading-snug`}
          >
            I’m building the missing products that people need.
          </p>
          <p className={`${paragraphStyles} ${secondaryColorStyles} mt-4`}>
            I design everything from software system to the user interface. I
            code religiously, take photos and practice witchcraft. I love
            open-source.
          </p>
          <div className="flex flex-row gap-6 mt-4">
            <Link href="/blog" className={linkStyles} prefetch={false}>
              Blog
            </Link>
            {/* <Link href="/photographs" className={linkStyles} prefetch={false} >
              Photographs
            </Link> */}
          </div>
          <p className={`${paragraphStyles} ${secondaryColorStyles} mt-4`}>
            &#119;&#111;&#114;&#107;&#064;&#104;&#104;&#097;&#105;&#046;&#100;&#101;&#118;
          </p>
        </div>
        <div className="flex flex-col max-w-[420px] gap-6">
          <p className={`${headingStyles} ${mainColorStyles} font-medium`}>
            Projects
          </p>
          <div className="flex flex-col gap-2">
            <p className={`${paragraphStyles} ${mainColorStyles}`}>
              2021 – Vietnamese sentiment analysis
            </p>
            <p className={`${paragraphStyles} ${secondaryColorStyles}`}>
              During my graduation thesis, I created a BERT-based model that
              tries to predict the sentiment of Vietnamese texts.{" "}
              <a
                href="https://huggingface.co/wonrax/phobert-base-vietnamese-sentiment"
                target="_blank"
                className={linkStyles}
              >
                Published on Hugging Face ↗
              </a>{" "}
              and has over 200,000 downloads.{" "}
              <a
                href="https://github.com/wonrax/phobert-base-vietnamese-sentiment"
                target="_blank"
                className={linkStyles}
              >
                GitHub ↗
              </a>
            </p>
          </div>
          <div className="flex flex-col gap-2">
            <p className={`${paragraphStyles} ${mainColorStyles}`}>
              2021 – Mybk Mobile Android app
            </p>
            <p className={`${paragraphStyles} ${secondaryColorStyles}`}>
              This is the app that I feel missing while attending college. Mybk
              Mobile helps you find your class schedules and other information
              quickly, even without the internet so you don’t miss your exam
              day. The app is{" "}
              <a
                href="https://play.google.com/store/apps/details?id=com.wonrax.mybk"
                target="_blank"
                className={linkStyles}
              >
                published on the Google Play Store
              </a>{" "}
              and has 1,500 monthly active users.{" "}
              <a
                href="https://github.com/wonrax/mybk-mobile"
                target="_blank"
                className={linkStyles}
              >
                GitHub ↗
              </a>
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}
