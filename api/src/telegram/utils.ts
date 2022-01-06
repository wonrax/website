import dotenv from "dotenv";

dotenv.config();

const telegramKey = process.env.TELEGRAM_KEY;
const telegramURL = process.env.TELEGRAM_URL;

const telegramPrefix = `${telegramURL}/bot${telegramKey}`;

export { telegramPrefix };
