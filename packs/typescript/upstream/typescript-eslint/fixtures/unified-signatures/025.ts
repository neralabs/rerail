
function p(key: string): Promise<string | undefined>;
function p(key: string, defaultValue: string): Promise<string>;
function p(key: string, defaultValue?: string): Promise<string | undefined> {
  const obj: Record<string, string> = {};
  return obj[key] || defaultValue;
}
    