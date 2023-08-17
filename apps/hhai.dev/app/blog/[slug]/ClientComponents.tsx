"use client";

export const ScrollToTopButton = () => {
  return (
    <button
      className="bg-gray-50 border rounded-full px-4 py-1"
      onClick={() =>
        document.documentElement.scrollTo({ top: 0, behavior: "smooth" })
      }
    >
      ↑ Top
    </button>
  );
};
