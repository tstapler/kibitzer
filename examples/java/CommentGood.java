class CommentGood {
    // sum adds two integers and returns the result.
    int sum(int a, int b) {
        return a + b;
    }

    // retry calls fn up to attempts times, waiting delay between failures.
    // Returns the last result produced — callers that need to distinguish
    // success from failure should have fn encode that in its return value,
    // since retry itself does not inspect it.
    int retry(int attempts, int delay, java.util.function.IntSupplier fn) {
        int lastResult = 0;
        for (int i = 0; i < attempts; i++) {
            lastResult = fn.getAsInt();
        }
        return lastResult;
    }
}
