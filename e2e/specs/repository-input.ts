/** Native WebDriver typing can drop newlines; exercise a multiline paste. */
export async function setRepositoryList(paths: string[]): Promise<void> {
  await browser.execute((value: string) => {
    const input = document.getElementById("repo") as HTMLTextAreaElement;
    input.value = value;
    input.dispatchEvent(new Event("input", { bubbles: true }));
  }, paths.join("\n"));
}
