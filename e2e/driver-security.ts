export interface VerificationResult {
  status: number | null;
  stderr?: Buffer | string;
  error?: Error;
}

export function assertVerificationSucceeded(result: VerificationResult): void {
  if (result.error || result.status !== 0) {
    const stderr = result.stderr?.toString().trim();
    throw new Error(
      `EdgeDriver signature validation failed: ${stderr || result.error?.message || "the verifier did not run"}`,
    );
  }
}
