import express, { Router } from "express";
import https from "https";
import fs from "fs";
import telegram from "./telegram/telegram";
import dotenv from "dotenv";
import setWebHook from "./telegram/setWebHook";
import { Http2ServerRequest } from "http2";

dotenv.config();

const app = express();
const port = parseInt(process.env.PORT, 10);

app.use(express.json());
app.use(express.urlencoded({ extended: true }));

const routes = Router();

const telegramRoute = "/telegram/" + process.env.TELEGRAM_KEY;
routes.route(telegramRoute).post(telegram.webhook);

// define a route handler for the default home page
app.get("/", (req, res) => {
  res.send("Hello world!");
});

app.use("/", routes);

const options = {
  key: fs.readFileSync(process.env.SSL_PRIVATE_KEY_PATH, "utf8"),
  cert: fs.readFileSync(process.env.SSL_CERT_PATH, "utf8"),
};

https.createServer(options, app).listen(port, () => {
  console.log(`server started at https://localhost:${port}`);
  setWebHook();
});
