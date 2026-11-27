import { WebClient } from "@slack/web-api";

const slack = new WebClient(process.env.SLACK_TOKEN);

export async function postToOps(text: string) {
  await slack.chat.postMessage({ channel: "#ops", text });
}
