import express, { Router } from "express";
import telegram from "./telegram/telegram";
import dotenv from "dotenv";
import setWebHook from "./telegram/setWebHook";

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

// start the Express server
app.listen(port, () => {
  console.log(`server started at http://localhost:${port}`);
  setWebHook();
});
