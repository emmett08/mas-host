"""Public errors distinguish host rejection from an unrecorded admitted effect."""


class HostError(RuntimeError):
    """The host rejected an operation or returned an invalid response."""


class EffectUncertain(HostError):
    """An effect was admitted without a recorded completion; reconcile it first."""
