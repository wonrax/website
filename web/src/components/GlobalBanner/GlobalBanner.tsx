import {
  createSignal,
  createEffect,
  Show,
  onCleanup,
  type JSXElement,
} from "solid-js";
import { Remarkable } from "remarkable";
import "./GlobalBanner.scss";

type BannerType = "announcement" | "alert" | "critical";

export interface BannerPayload {
  type: BannerType;
  content: string;
}

interface Props {
  bannerData?: BannerPayload | null;
}

// Extend window interface for TypeScript
declare global {
  interface Window {
    __bannerData?: BannerPayload;
  }
}

// Generate a simple hash for banner content to track dismissals
function generateContentHash(content: string): string {
  let hash = 0;
  for (let i = 0; i < content.length; i++) {
    const char = content.charCodeAt(i);
    hash = (hash << 5) - hash + char;
    hash = hash & hash; // Convert to 32-bit integer
  }
  return Math.abs(hash).toString(36);
}

interface DismissedBanner {
  hash: string;
  dismissedAt: number;
}

// Default expiration time: 30 days in milliseconds
const DISMISSAL_EXPIRY_MS = 30 * 24 * 60 * 60 * 1000;

// Clean up expired dismissed banners
function cleanupExpiredDismissals(): void {
  if (typeof window === "undefined") return;

  const dismissedBanners = localStorage.getItem("dismissedBanners");
  if (!dismissedBanners) return;

  try {
    const dismissed = JSON.parse(dismissedBanners);
    const now = Date.now();

    // Handle both old format (string[]) and new format (DismissedBanner[])
    let activeDismissals: DismissedBanner[] = [];

    if (Array.isArray(dismissed) && dismissed.length > 0) {
      if (typeof dismissed[0] === "string") {
        // Old format - convert to new format but mark as recently dismissed
        // to give them the full expiry period
        activeDismissals = dismissed.map((hash: string) => ({
          hash,
          dismissedAt: now,
        }));
      } else {
        // New format - filter out expired ones
        activeDismissals = dismissed.filter(
          (item: DismissedBanner) =>
            now - item.dismissedAt < DISMISSAL_EXPIRY_MS
        );
      }
    }

    localStorage.setItem("dismissedBanners", JSON.stringify(activeDismissals));
  } catch {
    // If parsing fails, clear the storage
    localStorage.removeItem("dismissedBanners");
  }
}

// Check if banner was previously dismissed and not expired
function wasBannerDismissed(content: string): boolean {
  if (typeof window === "undefined") return false;

  // Clean up expired dismissals first
  cleanupExpiredDismissals();

  const hash = generateContentHash(content);
  const dismissedBanners = localStorage.getItem("dismissedBanners");
  if (!dismissedBanners) return false;

  try {
    const dismissed: DismissedBanner[] = JSON.parse(dismissedBanners);
    const now = Date.now();

    return dismissed.some(
      (item) =>
        item.hash === hash && now - item.dismissedAt < DISMISSAL_EXPIRY_MS
    );
  } catch {
    return false;
  }
}

// Mark banner as dismissed with timestamp
function markBannerDismissed(content: string): void {
  if (typeof window === "undefined") return;

  // Clean up expired dismissals first
  cleanupExpiredDismissals();

  const hash = generateContentHash(content);
  const dismissedBanners = localStorage.getItem("dismissedBanners");
  let dismissed: DismissedBanner[] = [];

  if (dismissedBanners) {
    try {
      dismissed = JSON.parse(dismissedBanners);
    } catch {
      dismissed = [];
    }
  }

  // Check if already dismissed
  const existingIndex = dismissed.findIndex((item) => item.hash === hash);
  if (existingIndex >= 0) {
    // Update the timestamp
    dismissed[existingIndex].dismissedAt = Date.now();
  } else {
    // Add new dismissal
    dismissed.push({
      hash,
      dismissedAt: Date.now(),
    });
  }

  localStorage.setItem("dismissedBanners", JSON.stringify(dismissed));
}

export default function GlobalBanner(props: Props): JSXElement {
  const initialBannerData = () => props.bannerData || null;
  const [bannerData, setBannerData] = createSignal<BannerPayload | null>(
    initialBannerData()
  );
  const [isDismissed, setIsDismissed] = createSignal(false);

  const md = new Remarkable({
    html: false,
    xhtmlOut: false,
    breaks: true,
    langPrefix: "language-",
    typographer: false,
    quotes: "\"\"''",
  });

  // Check for existing banner data and listen for banner data events
  createEffect(() => {
    if (typeof window !== "undefined") {
      // Check if banner data was already stored on window (race condition fix)
      if (window.__bannerData && !bannerData()) {
        const data = window.__bannerData;
        // Check if this banner was previously dismissed
        if (!wasBannerDismissed(data.content)) {
          setBannerData(data);
        }
      }

      // Listen for new banner data events
      const handleBannerData = (event: CustomEvent) => {
        const data = event.detail;
        // Only show if not previously dismissed
        if (!wasBannerDismissed(data.content)) {
          setBannerData(data);
        }
        // Store on window as backup
        window.__bannerData = data;
      };

      window.addEventListener("banner-data", handleBannerData as EventListener);

      onCleanup(() => {
        window.removeEventListener(
          "banner-data",
          handleBannerData as EventListener
        );
      });
    }
  });

  const handleDismiss = () => {
    const data = bannerData();
    if (data) {
      markBannerDismissed(data.content);
    }
    setIsDismissed(true);
    // Clear stored banner data when dismissed
    if (typeof window !== "undefined") {
      delete window.__bannerData;
    }
  };

  const getBannerClass = (type: BannerType): string => {
    const baseClass = "global-banner";
    switch (type) {
      case "announcement":
        return `${baseClass} ${baseClass}--announcement`;
      case "alert":
        return `${baseClass} ${baseClass}--alert`;
      case "critical":
        return `${baseClass} ${baseClass}--critical`;
      default:
        return baseClass;
    }
  };

  return (
    <Show when={bannerData() && !isDismissed()}>
      <div class={getBannerClass(bannerData()!.type)}>
        <div class="global-banner__content">
          <div
            class="global-banner__text"
            // eslint-disable-next-line solid/no-innerhtml
            innerHTML={md.render(bannerData()!.content)}
          />
          <button
            class="global-banner__dismiss"
            onClick={handleDismiss}
            aria-label="Dismiss banner"
          >
            ×
          </button>
        </div>
      </div>
    </Show>
  );
}
