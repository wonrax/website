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

interface TikTokVideoInfo {
  status: string;
  detail: string;
  item: {
    id: string;
    desc: string;
    createTime: number;
    aweme_id: string;
    video: {
      height: number;
      width: number;
      duration: number;
      ratio: string;
      cover: string;
      originCover: string;
      dynamicCover: string;
      downloadAddr: [string];
      playAddr: [string];
    };
    author: {
      id: string;
      uniqueId: string;
      nickname: string;
      avatarThumb: string;
      avatarMedium: string;
      avatarLarger: string;
      signature: string;
      secUid: string;
    };
    music: {
      id: number;
      title: string;
      coverThumb: string;
      coverMedium: string;
      coverLarge: string;
      authorName: string;
    };
    stats: {
      commentCount: number;
      diggCount: number;
      playCount: number;
      shareCount: number;
    };
  };
}

function webhook(req: Request, res: Response) {
  res.send("success");

  const data: TextMessageUpdate = req.body;

  const tiktokRegex =
    /tiktok\.com\/@.+\/video\/.+?(\/|$)|.+?\.tiktok\.com\/.+?(\/|$)/g;

  const tiktokUrlMatch = data.message.text.match(tiktokRegex);

  if (tiktokUrlMatch) {
    handleTiktokDownload(`https://${tiktokUrlMatch[0]}`, data.message.chat.id);
  }
}

async function handleTiktokDownload(url: string, chatId: number) {
  const options: AxiosRequestConfig = {
    method: "GET",
    url: "https://video-nwm.p.rapidapi.com/url/",
    params: { url },
    headers: {
      "x-rapidapi-host": "video-nwm.p.rapidapi.com",
      "x-rapidapi-key": process.env.RAPID_API_TOKEN,
    },
  };

  try {
    const response = await axios(options);
    const data: TikTokVideoInfo = response.data;
    if (data.item.video.downloadAddr.length > 0) {
      const shortenedUrl = await shorten(data.item.video.downloadAddr[0]);
      const reply = `Video: ${data.item.desc}\nLink download: ${shortenedUrl}`;
      sendTelegramText(chatId, reply);
    } else {
      throw new Error("Video not found");
    }
  } catch (error) {
    sendTelegramText(
      chatId,
      "Can't download this video. Possible server fault, please try again later."
    );
  }
}

const telegram = { webhook };

export default telegram;
