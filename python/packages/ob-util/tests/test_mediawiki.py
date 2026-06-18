import warnings
import xml.etree.ElementTree as ET
from pathlib import Path

import pytest

from ob_util.parsers.mediawiki import (
    ContentChunk,
    MediawikiParser,
    ParseResult,
    _extract_all_revisions,
    blame_revisions,
    split_sections,
)


def _create_ob_dir(base: Path) -> Path:
    from ob.api import init as ob_init

    ob_init(ob_dir=base, force=True)
    return base


SINGLE_PAGE_XML = """\
<mediawiki>
  <page>
    <title>TestPage</title>
    <revision>
      <contributor><username>Alice</username><id>1</id></contributor>
      <timestamp>2024-01-15T10:30:00Z</timestamp>
      <text>Some content</text>
    </revision>
  </page>
</mediawiki>
"""

MULTI_PAGE_XML = """\
<mediawiki>
  <page>
    <title>PageOne</title>
    <revision>
      <contributor><username>Bob</username><id>2</id></contributor>
      <timestamp>2023-06-20T14:00:00Z</timestamp>
      <text>Content one</text>
    </revision>
  </page>
  <page>
    <title>PageTwo</title>
    <revision>
      <contributor><username>Charlie</username><id>3</id></contributor>
      <timestamp>2024-03-10T09:15:00Z</timestamp>
      <text>Content two</text>
    </revision>
  </page>
</mediawiki>
"""

MULTI_REVISION_XML = """\
<mediawiki>
  <page>
    <title>SharedPage</title>
    <revision>
      <contributor><username>Alice</username><id>1</id></contributor>
      <timestamp>2023-01-01T00:00:00Z</timestamp>
      <text>Old content</text>
    </revision>
    <revision>
      <contributor><username>Bob</username><id>2</id></contributor>
      <timestamp>2024-06-01T00:00:00Z</timestamp>
      <text>New content</text>
    </revision>
  </page>
</mediawiki>
"""

THREE_REVISION_TWO_AUTHOR_XML = """\
<mediawiki>
  <page>
    <title>北京</title>
    <revision>
      <contributor><username>Alice</username><id>1</id></contributor>
      <timestamp>2022-01-01T00:00:00Z</timestamp>
      <text>北京是中国的首都。
它有着悠久的历史。
这座城市拥有众多文化遗产。
</text>
    </revision>
    <revision>
      <contributor><username>Bob</username><id>2</id></contributor>
      <timestamp>2023-06-15T00:00:00Z</timestamp>
      <text>北京是中国的首都。
它有着悠久的历史。
这座城市拥有众多文化遗产。
北京的经济在近年来快速发展。
北京的GDP位居全国前列。
</text>
    </revision>
    <revision>
      <contributor><username>Alice</username><id>1</id></contributor>
      <timestamp>2024-03-20T00:00:00Z</timestamp>
      <text>北京是中国的首都。
它有着悠久的历史。
这座城市拥有丰富的文化遗产和现代化设施。
北京的经济在近年来快速发展。
北京的GDP位居全国前列。
</text>
    </revision>
  </page>
</mediawiki>
"""

MULTI_SECTION_PAGE_XML = """\
<mediawiki>
  <page>
    <title>北京</title>
    <revision>
      <contributor><username>Alice</username><id>1</id></contributor>
      <timestamp>2024-01-01T00:00:00Z</timestamp>
      <text>北京是中华人民共和国的首都，位于华北平原北部。它有着超过三千年的建城史和八百年的建都史。北京是世界上拥有世界文化遗产最多的城市之一。北京的历史可以追溯到西周时期，是中国四大古都之一。作为一座拥有深厚文化底蕴的古老城市，北京在全国乃至全世界都享有盛誉。

== 历史 ==
北京的历史可以追溯到西周时期。在元代，北京成为全国的政治中心。明清两代均以北京为首都，留下了故宫、天坛等著名历史遗迹。1949年中华人民共和国成立，定都北京，开启了新的历史篇章。北京拥有众多的历史遗迹和文化遗产，每年吸引数千万游客前来参观。故宫是世界上现存规模最大、保存最完整的木质结构古建筑群。天坛是明清两朝帝王祭天祈谷的场所，建筑精妙绝伦。颐和园是清朝的皇家园林，以昆明湖和万寿山为基础构建。长城是中华民族的象征，北京段长城以八达岭最为著名。圆明园虽然遭受破坏，但其遗址仍具有重要的历史价值。北京城池的格局历经多次变迁，每一段城墙都承载着独特的历史记忆。北京胡同和四合院是老北京文化的代表，体现了传统中国城市生活的特色。南锣鼓巷保留了完整的元代胡同格局，是北京最具特色的历史文化街区之一。什刹海地区汇集了众多的历史古迹和人文景观，是游客体验老北京风情的理想去处。北京的寺庙文化同样丰富多彩，雍和宫、白云观、法源寺等都是重要的宗教文化遗产。

== 地理 ==
北京位于华北平原的西北边缘，总面积约16410平方公里。气候为典型的暖温带半湿润大陆性季风气候，四季分明。北京的地势西北高、东南低，西部是太行山脉，北部是燕山山脉。永定河、潮白河等河流流经北京，为城市提供重要的水资源。北京的自然资源丰富，拥有多种矿产资源和大面积的森林覆盖。城市绿化覆盖率逐年提高，建成了大量的公园和绿地。北京周边的山区拥有丰富的动植物资源，是重要的生态屏障。密云水库是亚洲最大的人工湖之一，为城市提供清洁的饮用水源。北京的交通网络发达，拥有多条高速公路和铁路线路连接全国各地。首都国际机场和大兴国际机场是北京的两个主要航空枢纽。北京地铁系统是全球最繁忙的城市轨道交通系统之一，日客流量超过千万人次。

== 经济 ==
北京是中国重要的经济中心之一，服务业是北京经济的主要支柱。北京的高新技术产业发展迅速，中关村是中国最大的高科技产业区。金融街是中国的金融管理中心之一，聚集了大量金融机构总部。北京的GDP在全国城市中名列前茅，经济总量持续稳步增长。文化创意产业也是北京经济的重要组成部分，发展潜力巨大。北京证券交易所的设立进一步提升了城市的金融影响力。数字经济在北京蓬勃发展，众多互联网企业总部设于此。北京还拥有发达的会展经济，中国国际展览中心每年举办数百场大型展会。北京的商业区如王府井、西单、国贸等地是购物和消费的热门目的地。旅游业也是北京经济的重要支柱产业，每年接待国内外游客超过三亿人次。北京的科技创新能力在全国处于领先地位，拥有众多高校和科研院所。

== 参见 ==
* 上海
* 天津
* 广州

== 参考资料 ==
1. 北京市地方志编纂委员会
2. 中国统计年鉴

== 外部链接 ==
* [http://www.beijing.gov.cn 北京市人民政府]
</text>
    </revision>
  </page>
</mediawiki>
"""

SMALL_SECTION_PAGE_XML = """\
<mediawiki>
  <page>
    <title>小镇</title>
    <revision>
      <contributor><username>Alice</username><id>1</id></contributor>
      <timestamp>2024-01-01T00:00:00Z</timestamp>
      <text>这是一个很小的小镇介绍，内容不多。

== 历史 ==
这座小镇有着悠久的历史可以追溯到唐朝时期。在宋代成为重要的贸易中转站。明清两代繁荣发展。近代以来经历多次变迁，如今已成为旅游胜地。当地居民保留了传统的生活方式和手工艺技术。

== 概况 ==
这是一个小段落。
</text>
    </revision>
  </page>
</mediawiki>
"""

IP_CONTRIBUTOR_XML = """\
<mediawiki>
  <page>
    <title>IPPage</title>
    <revision>
      <contributor><ip>192.168.1.1</ip><id>0</id></contributor>
      <timestamp>2024-07-01T12:00:00Z</timestamp>
      <text>Anonymous edit</text>
    </revision>
  </page>
</mediawiki>
"""

NO_REVISION_XML = """\
<mediawiki>
  <page>
    <title>EmptyPage</title>
  </page>
</mediawiki>
"""

NO_CONTRIBUTOR_XML = """\
<mediawiki>
  <page>
    <title>NoContribPage</title>
    <revision>
      <timestamp>2024-01-01T00:00:00Z</timestamp>
      <text>Orphan content</text>
    </revision>
  </page>
</mediawiki>
"""

NS_XML = """\
<mediawiki xmlns="http://www.mediawiki.org/xml/export-0.11/">
  <page>
    <title>NSPage</title>
    <revision>
      <contributor><username>NSUser</username><id>10</id></contributor>
      <timestamp>2024-05-01T00:00:00Z</timestamp>
      <text>Namespaced content</text>
    </revision>
  </page>
</mediawiki>
"""

EMPTY_XML = "<mediawiki></mediawiki>"


# ---------------------------------------------------------------------------
# Original backward-compat tests
# ---------------------------------------------------------------------------


class TestMediawikiParserInit:
    def test_init_with_explicit_ob_dir(self, tmp_path):
        _create_ob_dir(tmp_path)
        p = MediawikiParser(ob_dir=tmp_path)
        assert p.ob_dir == tmp_path
        assert p.license == "CC-BY-SA-4.0"

    def test_init_with_custom_license(self, tmp_path):
        _create_ob_dir(tmp_path)
        p = MediawikiParser(ob_dir=tmp_path, license="MIT")
        assert p.license == "MIT"


class TestSinglePage:
    def test_parse_single_page(self, tmp_path):
        ob_dir = _create_ob_dir(tmp_path)
        xml_file = tmp_path / "dump.xml"
        xml_file.write_text(SINGLE_PAGE_XML)

        p = MediawikiParser(ob_dir=ob_dir)
        result = p.parse(str(xml_file))

        assert result.pages_parsed == 1
        assert result.authors_registered == 1
        assert result.sections_created == 1
        assert result.split_files_created == 0

    def test_author_registered(self, tmp_path):
        ob_dir = _create_ob_dir(tmp_path)
        xml_file = tmp_path / "dump.xml"
        xml_file.write_text(SINGLE_PAGE_XML)

        p = MediawikiParser(ob_dir=ob_dir)
        p.parse(str(xml_file))

        from ob.authors import list_all_authors

        authors = list_all_authors(ob_dir)
        assert len(authors) == 1
        assert authors[0]["name"] == "Alice"
        assert authors[0]["email"] == "Alice@mediawiki"

    def test_section_created(self, tmp_path):
        ob_dir = _create_ob_dir(tmp_path)
        xml_file = tmp_path / "dump.xml"
        xml_file.write_text(SINGLE_PAGE_XML)

        p = MediawikiParser(ob_dir=ob_dir)
        p.parse(str(xml_file))

        from ob.register import find_sections_by_path

        sections = find_sections_by_path(ob_dir, "raw/TestPage")
        assert len(sections) == 1
        assert sections[0]["license"] == "CC-BY-SA-4.0"
        assert sections[0]["year"] == "2024"


class TestMultiplePages:
    def test_parse_multiple_pages(self, tmp_path):
        ob_dir = _create_ob_dir(tmp_path)
        xml_file = tmp_path / "dump.xml"
        xml_file.write_text(MULTI_PAGE_XML)

        p = MediawikiParser(ob_dir=ob_dir)
        result = p.parse(str(xml_file))

        assert result.pages_parsed == 2
        assert result.authors_registered == 2
        assert result.sections_created == 2

    def test_both_sections_exist(self, tmp_path):
        ob_dir = _create_ob_dir(tmp_path)
        xml_file = tmp_path / "dump.xml"
        xml_file.write_text(MULTI_PAGE_XML)

        p = MediawikiParser(ob_dir=ob_dir)
        p.parse(str(xml_file))

        from ob.register import find_sections_by_path

        assert len(find_sections_by_path(ob_dir, "raw/PageOne")) == 1
        assert len(find_sections_by_path(ob_dir, "raw/PageTwo")) == 1


class TestMultiRevision:
    def test_both_authors_registered(self, tmp_path):
        ob_dir = _create_ob_dir(tmp_path)
        xml_file = tmp_path / "dump.xml"
        xml_file.write_text(MULTI_REVISION_XML)

        p = MediawikiParser(ob_dir=ob_dir)
        result = p.parse(str(xml_file))

        assert result.authors_registered == 2
        assert result.sections_created == 1

    def test_year_from_latest_revision(self, tmp_path):
        ob_dir = _create_ob_dir(tmp_path)
        xml_file = tmp_path / "dump.xml"
        xml_file.write_text(MULTI_REVISION_XML)

        p = MediawikiParser(ob_dir=ob_dir)
        p.parse(str(xml_file))

        from ob.register import find_sections_by_path

        sections = find_sections_by_path(ob_dir, "raw/SharedPage")
        assert sections[0]["year"] == "2024"


class TestIPContributor:
    def test_ip_used_as_author_name(self, tmp_path):
        ob_dir = _create_ob_dir(tmp_path)
        xml_file = tmp_path / "dump.xml"
        xml_file.write_text(IP_CONTRIBUTOR_XML)

        p = MediawikiParser(ob_dir=ob_dir)
        result = p.parse(str(xml_file))

        assert result.authors_registered == 1
        from ob.authors import list_all_authors

        authors = list_all_authors(ob_dir)
        assert authors[0]["name"] == "192.168.1.1"
        assert authors[0]["email"] == "192.168.1.1@mediawiki"


class TestEdgeCases:
    def test_no_revisions(self, tmp_path):
        ob_dir = _create_ob_dir(tmp_path)
        xml_file = tmp_path / "dump.xml"
        xml_file.write_text(NO_REVISION_XML)

        p = MediawikiParser(ob_dir=ob_dir)
        result = p.parse(str(xml_file))

        assert result.pages_parsed == 1
        assert result.authors_registered == 0
        assert result.sections_created == 0

    def test_no_contributor(self, tmp_path):
        ob_dir = _create_ob_dir(tmp_path)
        xml_file = tmp_path / "dump.xml"
        xml_file.write_text(NO_CONTRIBUTOR_XML)

        p = MediawikiParser(ob_dir=ob_dir)
        result = p.parse(str(xml_file))

        assert result.pages_parsed == 1
        assert result.authors_registered == 0
        assert result.sections_created == 0

    def test_empty_mediawiki(self, tmp_path):
        ob_dir = _create_ob_dir(tmp_path)
        xml_file = tmp_path / "dump.xml"
        xml_file.write_text(EMPTY_XML)

        p = MediawikiParser(ob_dir=ob_dir)
        result = p.parse(str(xml_file))

        assert result.pages_parsed == 0
        assert result.authors_registered == 0
        assert result.sections_created == 0


class TestNamespacedXML:
    def test_parse_with_namespace(self, tmp_path):
        ob_dir = _create_ob_dir(tmp_path)
        xml_file = tmp_path / "dump.xml"
        xml_file.write_text(NS_XML)

        p = MediawikiParser(ob_dir=ob_dir)
        result = p.parse(str(xml_file))

        assert result.pages_parsed == 1
        assert result.authors_registered == 1
        assert result.sections_created == 1


def _split_filename(title: str) -> str:
    import base64, hashlib

    raw = base64.urlsafe_b64encode(title.encode("utf-8")).decode("ascii")
    if len(raw) <= 200:
        return raw
    h = hashlib.sha256(title.encode("utf-8")).hexdigest()[:8]
    return raw[:191] + "_" + h


class TestSplit:
    def test_split_creates_files(self, tmp_path):
        ob_dir = _create_ob_dir(tmp_path)
        xml_file = tmp_path / "dump.xml"
        xml_file.write_text(MULTI_PAGE_XML)

        p = MediawikiParser(ob_dir=ob_dir)
        result = p.parse(str(xml_file), split=True)

        assert result.split_files_created == 2

        split_dir = ob_dir / ".ob" / "split"
        assert (split_dir / f"{_split_filename('PageOne')}.xml").exists()
        assert (split_dir / f"{_split_filename('PageTwo')}.xml").exists()

    def test_split_file_valid_xml(self, tmp_path):
        ob_dir = _create_ob_dir(tmp_path)
        xml_file = tmp_path / "dump.xml"
        xml_file.write_text(SINGLE_PAGE_XML)

        p = MediawikiParser(ob_dir=ob_dir)
        p.parse(str(xml_file), split=True)

        split_file = ob_dir / ".ob" / "split" / f"{_split_filename('TestPage')}.xml"
        tree = ET.parse(split_file)
        root = tree.getroot()
        assert root.tag == "mediawiki"
        page = root.find("page")
        assert page is not None
        assert page.find("title").text == "TestPage"


class TestIdempotency:
    def test_parsing_same_file_twice(self, tmp_path):
        ob_dir = _create_ob_dir(tmp_path)
        xml_file = tmp_path / "dump.xml"
        xml_file.write_text(SINGLE_PAGE_XML)

        p = MediawikiParser(ob_dir=ob_dir)
        r1 = p.parse(str(xml_file))
        r2 = p.parse(str(xml_file))

        assert r1.authors_registered == 1
        assert r2.authors_registered == 1

        from ob.authors import list_all_authors

        authors = list_all_authors(ob_dir)
        assert len(authors) == 1


class TestParseResult:
    def test_default_values(self):
        r = ParseResult()
        assert r.pages_parsed == 0
        assert r.authors_registered == 0
        assert r.sections_created == 0
        assert r.split_files_created == 0

    def test_custom_values(self):
        r = ParseResult(
            pages_parsed=5,
            authors_registered=3,
            sections_created=5,
            split_files_created=5,
        )
        assert r.pages_parsed == 5
        assert r.authors_registered == 3


# ---------------------------------------------------------------------------
# NEW: blame_revisions tests
# ---------------------------------------------------------------------------


class TestBlameRevisions:
    def test_single_revision(self):
        revs = [
            {"timestamp": "2024", "contributor": "Alice", "text": "Line 1\nLine 2\n"}
        ]
        result = blame_revisions(revs)
        assert len(result) == 2
        assert result[0] == ("Line 1\n", "Alice")
        assert result[1] == ("Line 2\n", "Alice")

    def test_two_revisions_full_replace(self):
        revs = [
            {"timestamp": "2023", "contributor": "Alice", "text": "Old line\n"},
            {"timestamp": "2024", "contributor": "Bob", "text": "New line\n"},
        ]
        result = blame_revisions(revs)
        assert len(result) == 1
        assert result[0] == ("New line\n", "Bob")

    def test_two_revisions_partial_edit(self):
        revs = [
            {
                "timestamp": "2023",
                "contributor": "Alice",
                "text": "Line 1\nLine 2\nLine 3\n",
            },
            {
                "timestamp": "2024",
                "contributor": "Bob",
                "text": "Line 1\nModified\nLine 3\n",
            },
        ]
        result = blame_revisions(revs)
        assert len(result) == 3
        assert result[0] == ("Line 1\n", "Alice")
        assert result[1] == ("Modified\n", "Bob")
        assert result[2] == ("Line 3\n", "Alice")

    def test_three_revisions_two_authors(self):
        revs = [
            {
                "timestamp": "2022",
                "contributor": "Alice",
                "text": "Line A\nLine B\nLine C\n",
            },
            {
                "timestamp": "2023",
                "contributor": "Bob",
                "text": "Line A\nLine B\nLine C\nLine D\nLine E\n",
            },
            {
                "timestamp": "2024",
                "contributor": "Alice",
                "text": "Line A\nLine B modified\nLine C\nLine D\nLine E\n",
            },
        ]
        result = blame_revisions(revs)
        assert len(result) == 5
        assert result[0] == ("Line A\n", "Alice")
        assert result[1] == ("Line B modified\n", "Alice")
        assert result[2] == ("Line C\n", "Alice")
        assert result[3] == ("Line D\n", "Bob")
        assert result[4] == ("Line E\n", "Bob")

    def test_sha1_skip(self):
        revs = [
            {
                "timestamp": "2023",
                "contributor": "Alice",
                "text": "Hello\n",
                "sha1": "abc123",
            },
            {
                "timestamp": "2024",
                "contributor": "Bob",
                "text": "Hello\n",
                "sha1": "abc123",
            },
        ]
        result = blame_revisions(revs)
        assert len(result) == 1
        assert result[0] == ("Hello\n", "Alice")

    def test_empty_revisions(self):
        assert blame_revisions([]) == []

    def test_empty_text(self):
        revs = [{"timestamp": "2024", "contributor": "Alice", "text": ""}]
        result = blame_revisions(revs)
        assert result == []

    def test_insert_only(self):
        revs = [
            {"timestamp": "2023", "contributor": "Alice", "text": "Line 1\n"},
            {
                "timestamp": "2024",
                "contributor": "Bob",
                "text": "Line 1\nLine 2\nLine 3\n",
            },
        ]
        result = blame_revisions(revs)
        assert len(result) == 3
        assert result[0] == ("Line 1\n", "Alice")
        assert result[1] == ("Line 2\n", "Bob")
        assert result[2] == ("Line 3\n", "Bob")

    def test_delete_lines(self):
        revs = [
            {
                "timestamp": "2023",
                "contributor": "Alice",
                "text": "Line 1\nLine 2\nLine 3\nLine 4\n",
            },
            {
                "timestamp": "2024",
                "contributor": "Bob",
                "text": "Line 1\nLine 3\n",
            },
        ]
        result = blame_revisions(revs)
        assert len(result) == 2
        assert result[0] == ("Line 1\n", "Alice")
        assert result[1] == ("Line 3\n", "Alice")


# ---------------------------------------------------------------------------
# NEW: split_sections tests
# ---------------------------------------------------------------------------


class TestSplitSections:
    def test_no_headings(self):
        text = "This is a simple page with no headings.\nJust plain text."
        chunks = split_sections(text, "TestPage")
        assert len(chunks) == 1
        assert chunks[0].heading == "[INTRO]"
        assert chunks[0].source_path == "raw/TestPage#[INTRO]"

    def test_with_headings(self):
        history = (
            "Some history content about the city that spans multiple paragraphs. "
            "The city was founded over a thousand years ago and has seen many changes. "
            "During the medieval period it became an important trade hub connecting east and west. "
            "In modern times the city has grown into a major metropolis with millions of residents. "
            "Historical landmarks dot the urban landscape reminding visitors of its rich past. "
            "Archaeological excavations have revealed artifacts dating back to ancient civilizations. "
            "The city walls built during the dynasty era still stand in parts of the old town. "
            "Many famous scholars and poets lived here throughout the centuries of its history."
        )
        geography = (
            "The geography of the region is diverse and fascinating for researchers. "
            "Mountains surround the western border while plains extend to the east. "
            "Several major rivers flow through the territory providing water for agriculture. "
            "The climate varies from subtropical in the south to continental in the north. "
            "Natural resources include minerals forests and freshwater sources. "
            "The coastal areas feature beautiful beaches that attract tourists year round. "
            "Wetlands in the river delta serve as important habitats for migratory birds. "
            "The highest peak reaches over three thousand meters above sea level."
        )
        text = (
            "Intro text here about this fascinating city and its many attractions.\n"
            "\n"
            "== History ==\n"
            f"{history}\n"
            "\n"
            "== Geography ==\n"
            f"{geography}\n"
        )
        chunks = split_sections(text, "TestPage")
        assert len(chunks) >= 2
        headings = [c.heading for c in chunks]
        assert any("History" in h for h in headings)
        assert any("Geography" in h for h in headings)

    def test_boilerplate_skipped(self):
        text = (
            "== 历史 ==\n"
            "Some history content that is long enough to pass the size threshold.\n"
            "More content here to make this section sufficiently large for testing.\n"
            "Additional padding to go past four hundred chars for the merge check.\n"
            "Even more padding to ensure we are well beyond the minimum threshold.\n"
            "\n"
            "== 参见 ==\n"
            "See also stuff\n"
            "\n"
            "== 参考资料 ==\n"
            "Reference stuff\n"
            "\n"
            "== 外部链接 ==\n"
            "External links here\n"
        )
        chunks = split_sections(text, "TestPage")
        headings = [c.heading for c in chunks]
        assert any("历史" in h for h in headings)
        assert not any("参见" in h for h in headings)
        assert not any("参考资料" in h for h in headings)
        assert not any("外部链接" in h for h in headings)

    def test_empty_wikitext(self):
        chunks = split_sections("", "TestPage")
        assert chunks == []

    def test_source_path_format(self):
        text = (
            "== 历史 ==\n"
            "Content about history that is long enough to pass the threshold size.\n"
            "More content to ensure the section is large enough for the merge check.\n"
            "Additional padding text to go well past four hundred chars minimum.\n"
            "Even more text to make absolutely sure we are above the threshold.\n"
        )
        chunks = split_sections(text, "北京")
        assert len(chunks) >= 1
        assert chunks[0].source_path.startswith("raw/北京#")

    def test_subheading_not_matched(self):
        text = (
            "== Section ==\n"
            "Main section content that is sufficiently long for the merge check.\n"
            "More content to pad the section well past the four hundred char minimum.\n"
            "Additional text to ensure we are above the threshold for merging.\n"
            "Final padding line to guarantee the section is large enough.\n"
            "\n"
            "=== SubSection ===\n"
            "Subsection content\n"
        )
        chunks = split_sections(text, "TestPage")
        assert len(chunks) >= 1
        first_text = chunks[0].raw_text
        assert "=== SubSection ===" in first_text

    def test_small_section_merged(self):
        text = (
            "== 历史 ==\n"
            "This is a long history section with lots of content to talk about.\n"
            "We need this section to be well over four hundred characters to avoid merging.\n"
            "So we keep adding more text about the history of this fascinating place.\n"
            "The history goes back thousands of years with many important events.\n"
            "Ancient civilizations thrived here and left behind remarkable artifacts.\n"
            "Modern archaeological discoveries continue to reveal new insights.\n"
            "\n"
            "== 概况 ==\n"
            "This is tiny.\n"
        )
        chunks = split_sections(text, "TestPage")
        merged_headings = [c.heading for c in chunks]
        assert any("概况" in h for h in merged_headings)
        small_chunk = [c for c in chunks if "概况" in c.heading][0]
        assert "历史" in small_chunk.heading


# ---------------------------------------------------------------------------
# NEW: _extract_all_revisions tests
# ---------------------------------------------------------------------------


class TestExtractAllRevisions:
    def test_multi_revision_xml(self):
        lines = MULTI_REVISION_XML.splitlines(keepends=True)
        info = _extract_all_revisions(lines)
        assert info is not None
        assert info["title"] == "SharedPage"
        assert len(info["revisions"]) == 2
        assert info["year"] == "2024"
        assert info["revisions"][0]["contributor"] == "Alice"
        assert info["revisions"][1]["contributor"] == "Bob"

    def test_single_revision_still_works(self):
        lines = SINGLE_PAGE_XML.splitlines(keepends=True)
        info = _extract_all_revisions(lines)
        assert info is not None
        assert info["title"] == "TestPage"
        assert len(info["revisions"]) == 1
        assert info["year"] == "2024"

    def test_no_revisions_returns_none(self):
        lines = NO_REVISION_XML.splitlines(keepends=True)
        info = _extract_all_revisions(lines)
        assert info is None

    def test_three_revisions_two_authors(self):
        lines = THREE_REVISION_TWO_AUTHOR_XML.splitlines(keepends=True)
        info = _extract_all_revisions(lines)
        assert info is not None
        assert info["title"] == "北京"
        assert len(info["revisions"]) == 3
        assert info["revisions"][0]["contributor"] == "Alice"
        assert info["revisions"][1]["contributor"] == "Bob"
        assert info["revisions"][2]["contributor"] == "Alice"

    def test_chunks_populated(self):
        lines = THREE_REVISION_TWO_AUTHOR_XML.splitlines(keepends=True)
        info = _extract_all_revisions(lines)
        assert info is not None
        assert isinstance(info["chunks"], list)
        assert len(info["chunks"]) >= 1

    def test_line_attributions_populated(self):
        lines = THREE_REVISION_TWO_AUTHOR_XML.splitlines(keepends=True)
        info = _extract_all_revisions(lines)
        assert info is not None
        attrs = info["line_attributions"]
        assert len(attrs) > 0
        authors_in_attrs = {a for _, a in attrs}
        assert "Alice" in authors_in_attrs
        assert "Bob" in authors_in_attrs


# ---------------------------------------------------------------------------
# NEW: Full integration with blame mode
# ---------------------------------------------------------------------------


class TestBlameMode:
    def test_blame_mode_with_sections(self, tmp_path):
        ob_dir = _create_ob_dir(tmp_path)
        xml_file = tmp_path / "dump.xml"
        xml_file.write_text(MULTI_SECTION_PAGE_XML)

        p = MediawikiParser(ob_dir=ob_dir)
        result = p.parse(str(xml_file), blame=True)

        assert result.pages_parsed == 1
        assert result.authors_registered == 1
        assert result.sections_created >= 2

        from ob.register import find_sections_by_path_prefix

        sections = find_sections_by_path_prefix(ob_dir, "raw/北京")
        assert len(sections) >= 2

    def test_blame_mode_multi_revision_per_chunk_authors(self, tmp_path):
        ob_dir = _create_ob_dir(tmp_path)
        xml_file = tmp_path / "dump.xml"
        xml_file.write_text(THREE_REVISION_TWO_AUTHOR_XML)

        p = MediawikiParser(ob_dir=ob_dir)
        result = p.parse(str(xml_file), blame=True)

        assert result.pages_parsed == 1
        assert result.authors_registered == 2

    def test_blame_mode_backward_compat_single_rev(self, tmp_path):
        ob_dir = _create_ob_dir(tmp_path)
        xml_file = tmp_path / "dump.xml"
        xml_file.write_text(SINGLE_PAGE_XML)

        p = MediawikiParser(ob_dir=ob_dir)
        result = p.parse(str(xml_file), blame=True)

        assert result.pages_parsed == 1
        assert result.authors_registered == 1
        assert result.sections_created >= 1

    def test_blame_mode_creates_prefixed_sections(self, tmp_path):
        ob_dir = _create_ob_dir(tmp_path)
        xml_file = tmp_path / "dump.xml"
        xml_file.write_text(MULTI_SECTION_PAGE_XML)

        p = MediawikiParser(ob_dir=ob_dir)
        p.parse(str(xml_file), blame=True)

        from ob.register import find_sections_by_path_prefix

        sections = find_sections_by_path_prefix(ob_dir, "raw/北京")
        section_paths = [s["path"] for s in sections]
        assert any("#" in p for p in section_paths)


# ---------------------------------------------------------------------------
# NEW: OOM prevention test
# ---------------------------------------------------------------------------


class TestOOMPrevention:
    def test_too_many_revisions_skipped(self):
        revs = [
            {"timestamp": f"2024-{i:04d}", "contributor": "Bot", "text": f"Rev {i}\n"}
            for i in range(1001)
        ]
        lines = ["<page>", "<title>BigPage</title>", "<ns>0</ns>"]
        for i, r in enumerate(revs):
            lines.append(f"<revision>")
            lines.append(f"<contributor><username>{r['contributor']}</username></contributor>")
            lines.append(f"<timestamp>{r['timestamp']}T00:00:00Z</timestamp>")
            lines.append(f"<text>{r['text']}</text>")
            lines.append(f"</revision>")
        lines.append("</page>")

        with warnings.catch_warnings(record=True) as w:
            warnings.simplefilter("always")
            info = _extract_all_revisions(lines)
            assert info is None
            assert any("1001" in str(warning.message) for warning in w)


# ---------------------------------------------------------------------------
# NEW: ContentChunk dataclass tests
# ---------------------------------------------------------------------------


class TestContentChunk:
    def test_default_values(self):
        chunk = ContentChunk(page_title="Test", heading="H", text="t", raw_text="r")
        assert chunk.authors == []
        assert chunk.year == ""
        assert chunk.start_line == 0
        assert chunk.end_line == 0
        assert chunk.source_path == ""

    def test_field_assignment(self):
        chunk = ContentChunk(
            page_title="北京",
            heading="历史",
            text="clean",
            raw_text="raw",
            authors=["Alice", "Bob"],
            year="2024",
            start_line=5,
            end_line=10,
            source_path="raw/北京#历史",
        )
        assert chunk.page_title == "北京"
        assert chunk.heading == "历史"
        assert len(chunk.authors) == 2
        assert chunk.start_line == 5
        assert chunk.end_line == 10


# ---------------------------------------------------------------------------
# NEW: find_sections_by_path_prefix test
# ---------------------------------------------------------------------------


class TestFindSectionsByPathPrefix:
    def test_prefix_match(self, tmp_path):
        ob_dir = _create_ob_dir(tmp_path)

        from ob.api import author_add, register_section

        aid = author_add(name="Alice", email="alice@test.com", ob_dir=ob_dir)
        register_section(
            ob_dir=ob_dir,
            path="raw/北京#历史",
            authors=[aid],
            license="CC-BY-SA-4.0",
            year="2024",
        )
        register_section(
            ob_dir=ob_dir,
            path="raw/北京#地理",
            authors=[aid],
            license="CC-BY-SA-4.0",
            year="2024",
        )
        register_section(
            ob_dir=ob_dir,
            path="raw/上海#历史",
            authors=[aid],
            license="CC-BY-SA-4.0",
            year="2024",
        )

        from ob.register import find_sections_by_path_prefix

        results = find_sections_by_path_prefix(ob_dir, "raw/北京")
        assert len(results) == 2
        paths = [r["path"] for r in results]
        assert "raw/北京#历史" in paths
        assert "raw/北京#地理" in paths
        assert "raw/上海#历史" not in paths

    def test_no_match(self, tmp_path):
        ob_dir = _create_ob_dir(tmp_path)

        from ob.register import find_sections_by_path_prefix

        results = find_sections_by_path_prefix(ob_dir, "raw/不存在")
        assert results == []
