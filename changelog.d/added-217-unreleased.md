- The server's `script` method runs a program in shed's JavaScript subset beside the terminals: reads are
  called as they are, and every call that acts is first asked of the guard named in `[script]` of the
  config file (refused when none is named); the reply lists every acting call with its result.
