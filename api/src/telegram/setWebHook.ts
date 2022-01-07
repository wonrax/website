import axios from "axios";
import { telegramPrefix } from "./utils";

function setWebHook() {
  const telegramKey = process.env.TELEGRAM_KEY;
  const domainURL = process.env.DOMAIN_URL;

  const webhookRoute = domainURL + "/telegram/" + telegramKey;

  const url = `${telegramPrefix}/setWebhook`;

  axios
    .get(url, {
      params: {
        url: webhookRoute,
      },
    })
    .then((response) => {
      console.log(
        "Telegram webhook set successfully. Response:",
        response.data
      );
    })
    .catch((error) => {
      console.log(
        "Can not set telegram webhook. Response:",
        error?.response?.data
      );
    });
}

export default setWebHook;
