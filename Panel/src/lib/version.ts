import packageJson from "../../package.json";

export const APP_SEMVER = packageJson.version;

export function displayVersion(version: string): string {
  return version;
}

export const APP_DISPLAY_VERSION = displayVersion(APP_SEMVER);
