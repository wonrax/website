import axios, { AxiosRequestConfig } from "axios";
import dotenv from "dotenv";

dotenv.config();

const headers = {
  "Content-Type": "application/json",
  apikey: process.env.BRANDLY_API_KEY,
  workspace: process.env.BRANDLY_TIKTOK_WORKSPACE,
};

const shorten = async (url: string) => {
  const endpoint = "https://api.rebrandly.com/v1/links";
  const linkRequest = {
    destination: url,
    domain: { fullName: "rebrand.ly" },
    // , slashtag: "A_NEW_SLASHTAG"
    // , title: "Rebrandly YouTube channel"
  };
  const apiCall: AxiosRequestConfig = {
    method: "post",
    url: endpoint,
    data: linkRequest,
    headers,
  };
  const apiResponse = await axios(apiCall);
  const link = apiResponse.data;
  return link.shortUrl;
};

export default shorten;
