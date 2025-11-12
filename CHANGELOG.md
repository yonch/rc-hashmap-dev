# Changelog

## 0.1.0 (2025-11-12)


### Features

* add accessors and iterators to CountedHashMap ([37d9e78](https://github.com/yonch/rc-hashmap-dev/commit/37d9e784c76d6be35fc60ed20fbd6013333d4135))
* add DebugReentrancy to check re-entrancy contraints ([607153d](https://github.com/yonch/rc-hashmap-dev/commit/607153d3ffb87b9611af76c570e948e2779f4dd0))
* add default hash selection, switch to wyhash ([f4b6dab](https://github.com/yonch/rc-hashmap-dev/commit/f4b6dabb3e8b0b6226c1cf2409527aca2876d458))
* add find&lt;Q&gt;(); make CountedHashMap::put take &mut self ([56f19d1](https://github.com/yonch/rc-hashmap-dev/commit/56f19d14a70ebba429ce1dc3410b48d33c0c701f))
* add insert_with() in lower_level, simplifies RcHashMap::insert() ([0350dc9](https://github.com/yonch/rc-hashmap-dev/commit/0350dc9c03dc30fa245db82144a1e89a2ab29a2f))
* add ManualRc to increment/decrement Rc strong counts ([8da7998](https://github.com/yonch/rc-hashmap-dev/commit/8da79985a4b821038d7149a76d8106992163cf83))
* add RcHashMap ([40b1d5e](https://github.com/yonch/rc-hashmap-dev/commit/40b1d5e146d8f2bbcaf5f300fcb5f175c7e2f07e))
* **CountedHashMap:** implement concrete iterators ([f406e7f](https://github.com/yonch/rc-hashmap-dev/commit/f406e7f42940d72e4eae1dde5674b36f7581fc0e))
* **HandleHashMap:** add concrete iterator types ([0cf9d1e](https://github.com/yonch/rc-hashmap-dev/commit/0cf9d1e0b4bd0b3a5b520def9689b4ccc15ced47))
* implement iterators and Ref accessors ([4174458](https://github.com/yonch/rc-hashmap-dev/commit/4174458c138c1926c993c9499ee14c10b24a037d))
* **RcHashMap:** simplify owner comparison ([bd9c693](https://github.com/yonch/rc-hashmap-dev/commit/bd9c69333bfb203c0f83ce0d3e9a984378bb35a2))
* **RcHashMap:** use ManuallyDrop in Ref to tighten error handling ([3cdc85a](https://github.com/yonch/rc-hashmap-dev/commit/3cdc85a3fe427268d99627a4ae8d7a4fb3bbc11f))
* return 'static CountedHandle; reduce unsafe ([4d0dd1e](https://github.com/yonch/rc-hashmap-dev/commit/4d0dd1e5f6481f623fff008dce8336fac82c0b96))
* use more intuitive method and accessors names ([5e4a9b3](https://github.com/yonch/rc-hashmap-dev/commit/5e4a9b3eb18a90e21b7e91380bfcf54643e2bfc2))


### Bug Fixes

* add mutable annotations in benchmark ([8ff4a64](https://github.com/yonch/rc-hashmap-dev/commit/8ff4a6406a48d35731775a1612e0567a69ccb165))
* **benchmark:** add stricter checks that values exist in access benchmarks ([09de487](https://github.com/yonch/rc-hashmap-dev/commit/09de487f0b7a904fedf84bac016a21beb2dc7c8f))
* CountedHashMap and RcHashMap issues ([53ab8ac](https://github.com/yonch/rc-hashmap-dev/commit/53ab8ac7548daa621c2aeaf195e59bfa540e574d))
* iai benchmarks ([c3c1cdc](https://github.com/yonch/rc-hashmap-dev/commit/c3c1cdc115fc5347242a406a3fad12c4324185c8))
* panic on stale handles in put ([ea94b80](https://github.com/yonch/rc-hashmap-dev/commit/ea94b80f18083edc9d426ff8a6e9e730475acc71))


### Performance Improvements

* add iai benchmarks for access through refs and iter ([bb2a107](https://github.com/yonch/rc-hashmap-dev/commit/bb2a107d07ea25bba29e0f29cc299407ccc28c08))
* add iai-based benchmarks ([6e961cf](https://github.com/yonch/rc-hashmap-dev/commit/6e961cfd92489b56a51acd036f9ea59b4b46c3be))
* do not measure setup in iai benchmarks, add some mesaured ops ([3c038a1](https://github.com/yonch/rc-hashmap-dev/commit/3c038a1ed8d39e74e3863c726a8eb0c300bda89d))
* enable HandleHashMap benchmarks by default ([ae6758d](https://github.com/yonch/rc-hashmap-dev/commit/ae6758d47632620b87f3fad42b00240200426d0b))
* expand criterion benchmark, replace iai, add CountedHashMap bench ([5a1f49e](https://github.com/yonch/rc-hashmap-dev/commit/5a1f49e0be8115940448b2b08d3b5e0a5d5137a0))
* fix remove benchmarks to not reorder chosen indices ([5840005](https://github.com/yonch/rc-hashmap-dev/commit/5840005fb28c0ed8765c193349bc803e008f077d))
* increase benchmark sample size for less noise ([76abc09](https://github.com/yonch/rc-hashmap-dev/commit/76abc096214863bf5293856c5fcf92e0a1216a5c))
* migrate benchmarks to criterion 0.7 ([665d21c](https://github.com/yonch/rc-hashmap-dev/commit/665d21c3ba483045dc3da4ed9f39ec185b3d2840))
* optimize insert happy path ([b847be3](https://github.com/yonch/rc-hashmap-dev/commit/b847be3eb814f5dac6f10e5aa4ff1e3b4ed2aff0))
* output throughput in benchmarks ([9ffecf1](https://github.com/yonch/rc-hashmap-dev/commit/9ffecf1ebc872396ec1bed3aa805b80b5fca98b4))
* pre-compute benchmark random access lists ([305a412](https://github.com/yonch/rc-hashmap-dev/commit/305a41269968769a934bdd7a611547e831c255db))
* standardize iai benchmarks on 1000 ops ([30a5a22](https://github.com/yonch/rc-hashmap-dev/commit/30a5a2250d9e2936f3ccde8c830d93ebdcd2744b))
* standardize on rand_pcg for benchmark pseudo-random generation ([431b622](https://github.com/yonch/rc-hashmap-dev/commit/431b62224ed90e1bde0c4eb581b5ec75d9ab4cc3))
