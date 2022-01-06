import dotenv from "dotenv";
import axios from "axios";

dotenv.config();

const telegramKey = process.env.TELEGRAM_KEY;
const telegramURL = process.env.TELEGRAM_URL;

const telegramPrefix = `${telegramURL}/bot${telegramKey}`;

const telegramSendTextEndpoint = `${telegramPrefix}/sendMessage`;

const sendTelegramText = async (chatId: number, reply: string) => {
  axios
    .get(telegramSendTextEndpoint, {
      params: {
        chat_id: chatId,
        text: reply,
      },
    })
    .catch((error) => {
      console.log(error);
    });
};

export { telegramPrefix, sendTelegramText };
