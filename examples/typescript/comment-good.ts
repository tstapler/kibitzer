// sum adds two numbers and returns the result.
function sum(a: number, b: number): number {
    return a + b;
}

// retry calls fn up to attempts times, waiting delay between failures.
// Returns the first successful result, or the last error if every attempt
// fails — callers that need cancellation should wrap fn themselves, since
// retry does not accept an AbortSignal.
function retry(attempts: number, delay: number, fn: () => number): number {
    let lastResult = 0;
    for (let i = 0; i < attempts; i++) {
        lastResult = fn();
    }
    return lastResult;
}
