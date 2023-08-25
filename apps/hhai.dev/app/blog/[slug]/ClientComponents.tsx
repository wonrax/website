"use client";

export const ScrollToTopButton = () => {
  return (
    <button
      className="bg-gray-100 rounded-full px-4 py-1 text-gray-700"
      onClick={() =>
        document.documentElement.scrollTo({ top: 0, behavior: "smooth" })
      }
    >
      ↑ Top
    </button>
  );
};
