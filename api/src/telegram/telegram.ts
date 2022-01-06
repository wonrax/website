import { Request, Response } from "express";
import { telegramPrefix } from "./utils";
import axios from "axios";

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
  const responseURL = `${telegramPrefix}/sendMessage`;
  axios.get(responseURL, {
    params: {
      chat_id: data.message.chat.id,
      text: "Got it babe!",
    },
  });
}

const telegram = { webhook };

export default telegram;
