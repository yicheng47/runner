#!/bin/sh
# Synthetic fixture source: the glyph classes behind the xterm garble
# bug class (#307) and CJK width bugs. Recorded through a PTY by
# record-fixture so the byte log is a real terminal stream.
printf 'ascii   |abcdefghij|\n'
printf 'cjk     |中文宽度测试|\n'
printf 'cjk-mix |a中b文c宽d|\n'
printf 'fullwid |ＡＢＣ１２３，。|\n'
printf 'kana    |テスト、ひらがな|\n'
printf 'hangul  |한국어테스트|\n'
printf 'emoji   |😀🎉🚀🔥|\n'
printf 'skintone|👍🏽👋🏿|\n'
printf 'zwj     |👨‍👩‍👧‍👦👩‍💻|\n'
printf 'flag    |🇨🇳🇺🇸|\n'
printf 'combine |e\xcc\x81a\xcc\x88n\xcc\x83|\n'
printf 'box     |\xe2\x94\x8c\xe2\x94\x80\xe2\x94\xac\xe2\x94\x80\xe2\x94\x90|\n'
printf 'box2    |\xe2\x94\x9c\xe2\x94\x80\xe2\x94\xbc\xe2\x94\x80\xe2\x94\xa4|\n'
printf 'box3    |\xe2\x94\x94\xe2\x94\x80\xe2\x94\xb4\xe2\x94\x80\xe2\x94\x98|\n'
printf 'rounded |\xe2\x95\xad\xe2\x95\xae\xe2\x95\xb0\xe2\x95\xaf|\n'
printf 'double  |\xe2\x95\x94\xe2\x95\x90\xe2\x95\xa6\xe2\x95\x97\xe2\x95\x91|\n'
printf 'blocks  |\xe2\x96\x88\xe2\x96\x93\xe2\x96\x92\xe2\x96\x91\xe2\x96\x8c\xe2\x96\x90|\n'
printf 'braille |\xe2\xa0\x8b\xe2\xa0\x99\xe2\xa0\xb9\xe2\xa0\xb8\xe2\xa0\xbc|\n'
# SGR runs: color + bold/italic/underline across wide chars.
printf 'sgr     |\033[31m红\033[0m\033[1;32mbold绿\033[0m\033[4;34m下划线\033[0m|\n'
printf 'sgr256  |\033[38;5;208morange\033[0m\033[48;5;19m蓝底\033[0m|\n'
printf 'sgrtrue |\033[38;2;255;100;0mtruecolor\033[0m|\n'
# Wide char at the last column: forces the leading-spacer path.
printf '%78s中\n' ''
printf 'end     |done|\n'
