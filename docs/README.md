# shrinkfit documentation

New here? Start with the [project README](../README.md) for what shrinkfit does and a five-minute start.
These pages go deeper. Read what you need, in any order.

## Using shrinkfit

| Page | What it covers |
|---|---|
| [Options and report](options.md) | Every option in every language, what the report tells you, and protect patterns. |
| [Using shrinkfit in an agent](agents.md) | The checklist for tools that return big results to a model: strategy, exact identifiers, headroom, notes to the model, a complete example. |
| [Model scoring with Jev](model-scoring.md) | Let a decision model choose which fields to keep: how to turn it on, the evidence, what is sent, what it costs, what happens when it fails. |
| [Command-line guide](cli.md) | Every flag, the stderr notice for agents, exit codes and the local playground. |

## Understanding and changing shrinkfit

| Page | What it covers |
|---|---|
| [How it works](ALGORITHM.md) | How the budget is spent: sizes, objects, arrays, strings, the `condense` strategy, verification. |
| [Extending shrinkfit](EXTENDING.md) | Adding an array strategy or a field scorer. |
| [Architecture and development](architecture.md) | Repository layout, the C ABI the bindings share, cargo features, building and testing. |
| [Path-selection experiment](../experiments/path-selection/README.md) | The data, labelling protocol, scripts and results behind model scoring. |
| [Releasing](../RELEASING.md) | How a release is cut and published. |
