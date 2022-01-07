import { Request, Response } from "express";
import axios, { AxiosRequestConfig } from "axios";
import { sendTelegramText } from "./utils";
import dotenv from "dotenv";
import shorten from "./rebrandly";

dotenv.config();

interface TextMessageUpdate {
  update_id: number;
  message: {
    message_id: number;
    from: {
      id: number;
      is_bot: boolean;
      first_name: string;
      username: string;
      language_code: string;
    };
    chat: {
      id: number;
      first_name: string;
      username: string;
      type: string;
    };
    date: number;
    text: string;
  };
}

function webhook(req: Request, res: Response) {
  res.send("success");

  const data: TextMessageUpdate = req.body;
  handleIncomingMessage(data);
}

async function handleIncomingMessage(data: TextMessageUpdate) {
  if (!data.message?.text) {
    return;
  }

  const tiktokRegex =
    /tiktok\.com\/@.+\/video\/\d+|[a-z]+\.tiktok\.com\/([A-Za-z]|\d)+(\/|$|\s|\n)/g;

  const tiktokUrlMatch = data.message.text.match(tiktokRegex);

  if (tiktokUrlMatch) {
    handleTiktokDownload(`https://${tiktokUrlMatch[0]}`, data.message.chat.id);
  }

  const twitterRegex = /twitter\.com\/.+\/status\/\d+/g;
  const twitterUrlMatch = data.message.text.match(twitterRegex);

  if (twitterUrlMatch) {
    handleTweetDownload(`https://${twitterUrlMatch[0]}`, data.message.chat.id);
  }

  // https://www.reddit.com/r/okbuddyretard/comments/rxx60r/3_points_to_muggledorf/?utm_source=share&utm_medium=web2x&context=3
  const redditRegex = /reddit\.com\/r\/.+\/comments\/.+?($|\s|\n|\/)/g;
  const redditUrlMatch = data.message.text.match(redditRegex);

  if (redditUrlMatch) {
    handleRedditDownload(`https://${redditUrlMatch[0]}`, data.message.chat.id);
  }
}

interface TikTokVideoInfo {
  status: string;
  data: {
    desc: string;
    video: {
      download_addr: {
        url_list: string[];
      };
      play_addr: {
        url_list: string[];
      };
    };
  };
}

async function handleTiktokDownload(url: string, chatId: number) {
  sendTelegramText(chatId, "🏎️ Processing...\nThis may take up to 10 seconds.");

  const options: AxiosRequestConfig = {
    method: "GET",
    url: "https://tiktok-best-experience.p.rapidapi.com/",
    params: { url },
    headers: {
      "x-rapidapi-host": "tiktok-best-experience.p.rapidapi.com",
      "x-rapidapi-key": process.env.RAPID_API_TOKEN,
    },
    timeout: 10000,
  };

  try {
    const response = await axios(options);
    const resData: TikTokVideoInfo = response.data;

    if (resData.status !== "ok") {
      throw new Error("TikTok API returned an error");
    }

    let shortenedWMUrl = null;
    let shortenedNoWMUrl = null;

    if (resData.data.video.download_addr.url_list.length > 0) {
      shortenedWMUrl = await shorten(
        resData.data.video.download_addr.url_list[0]
      );
    }
    if (resData.data.video.play_addr.url_list.length > 0) {
      shortenedNoWMUrl = await shorten(
        resData.data.video.play_addr.url_list[0]
      );
    }
    const reply =
      `Video: ${resData.data.desc}\n\n` +
      `No watermark video: ${shortenedNoWMUrl || "Couldn't find"}\n\n` +
      `Watermark video: ${shortenedWMUrl || "Couldn't find"}`;

    sendTelegramText(chatId, reply);
  } catch (error) {
    if (axios.isAxiosError(error) && error.response) {
      console.log(`Cannot get Tiktok URL ${url}`, error.response?.data);
    }
    sendTelegramText(
      chatId,
      "Can't download this video. Possible server fault, please try again later."
    );
  }
}

interface TweetVideoInfo {
  id: string;
  url: {
    url: string;
    quality: string;
  }[];
  meta: {
    title: string;
  };
  video_quality: [string];
  thumb: string;
}

async function handleTweetDownload(url: string, chatId: number) {
  sendTelegramText(chatId, "🏎️ Processing...\nThis may take up to 10 seconds.");

  const options: AxiosRequestConfig = {
    method: "POST",
    url: "https://twitter65.p.rapidapi.com/api/twitter/links",
    data: { url },
    headers: {
      "x-rapidapi-host": "twitter65.p.rapidapi.com",
      "x-rapidapi-key": process.env.RAPID_API_TOKEN,
    },
    timeout: 10000,
  };

  try {
    const response = await axios(options);
    const resData: TweetVideoInfo = response.data;

    if (!resData.id) {
      throw new Error("No video found in tweet");
    }

    if (resData.url.length === 0) {
      throw new Error("No video found in tweet");
    }

    let videoUrl = null;
    let highestQuality = 0;
    for (const urlObject of resData.url) {
      const quality = parseInt(urlObject.quality, 10);
      if (quality > highestQuality) {
        highestQuality = quality;
        videoUrl = urlObject.url;
      }
    }

    if (videoUrl === null) {
      throw new Error("No video found in tweet");
    }

    const shortenedUrl = await shorten(videoUrl);

    const reply =
      `Video: ${resData.meta.title}\n\n` + `Download link: ${shortenedUrl}`;

    sendTelegramText(chatId, reply);
  } catch (error) {
    if (axios.isAxiosError(error) && error.response) {
      console.log(`Cannot get Tweet video from ${url}`, error.response?.data);
    }
    sendTelegramText(
      chatId,
      "Can't download this video. The link could be incorrect (not a video tweet) or the server is down. Try again later."
    );
  }
}

async function handleRedditDownload(url: string, chatId: number) {
  sendTelegramText(chatId, `Download video: https://savemp4.red/?url=${url}`);
}

const telegram = { webhook };

export default telegram;
