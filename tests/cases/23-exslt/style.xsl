<?xml version="1.0"?>
<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
    xmlns:exsl="http://exslt.org/common" xmlns:set="http://exslt.org/sets"
    xmlns:str="http://exslt.org/strings" xmlns:math="http://exslt.org/math"
    xmlns:saxon="http://icl.com/saxon"
    extension-element-prefixes="exsl">
  <xsl:output method="text"/>
  <xsl:variable name="rtf"><a>1</a><b>2</b><a>3</a></xsl:variable>
  <xsl:template match="/">
    <xsl:variable name="ns" select="/r/n"/>
    <xsl:text>rtf: </xsl:text><xsl:value-of select="count(exsl:node-set($rtf)/a)"/>,<xsl:value-of select="exsl:node-set($rtf)/b"/>,<xsl:value-of select="count(saxon:node-set($rtf)/*)"/>
    <xsl:text>&#10;string: </xsl:text><xsl:value-of select="count(exsl:node-set('abc'))"/>,<xsl:value-of select="exsl:node-set('abc')"/>,<xsl:value-of select="count(exsl:node-set(''))"/>,<xsl:value-of select="exsl:node-set(1 div 0)"/>,<xsl:value-of select="name(exsl:node-set('x'))"/>
    <xsl:text>&#10;type: </xsl:text><xsl:value-of select="exsl:object-type('s')"/>,<xsl:value-of select="exsl:object-type(1)"/>,<xsl:value-of select="exsl:object-type(true())"/>,<xsl:value-of select="exsl:object-type($ns)"/>
    <xsl:text>&#10;sets: </xsl:text><xsl:value-of select="count(set:difference($ns, $ns[1]))"/>,<xsl:value-of select="count(set:intersection($ns, $ns[position() > 2]))"/>,<xsl:for-each select="set:distinct(/r/n)"><xsl:value-of select="."/></xsl:for-each>,<xsl:value-of select="set:has-same-node($ns, /r/n[3])"/>,<xsl:value-of select="set:has-same-node($ns, /r/s)"/>
    <xsl:text>&#10;leading: </xsl:text><xsl:value-of select="count(set:leading($ns, $ns[3]))"/>,<xsl:value-of select="count(set:leading($ns, /r/s))"/>,<xsl:value-of select="count(set:leading($ns, /r/nothing))"/>,<xsl:value-of select="count(set:trailing($ns, $ns[3]))"/>,<xsl:value-of select="count(set:trailing($ns, $ns[3] | $ns[4]))"/>
    <xsl:text>&#10;tokenize: </xsl:text><xsl:for-each select="str:tokenize(' a,b  c;;d ', ' ,;')"><xsl:value-of select="name()"/>[<xsl:value-of select="."/>]</xsl:for-each>|<xsl:for-each select="str:tokenize('xyz', '')">[<xsl:value-of select="."/>]</xsl:for-each>|<xsl:value-of select="count(str:tokenize('a b'))"/>
    <xsl:text>&#10;split: </xsl:text><xsl:for-each select="str:split('a, b,, cAND d', ',')">[<xsl:value-of select="."/>]</xsl:for-each>|<xsl:for-each select="str:split('xANDyandz', 'and')">[<xsl:value-of select="."/>]</xsl:for-each>|<xsl:for-each select="str:split('ab', '')">[<xsl:value-of select="."/>]</xsl:for-each>
    <xsl:text>&#10;concat: </xsl:text><xsl:value-of select="str:concat(/r/s)"/>
    <xsl:text>&#10;padding: </xsl:text>[<xsl:value-of select="str:padding(5)"/>][<xsl:value-of select="str:padding(5, 'ab')"/>][<xsl:value-of select="str:padding(2.7, 'xyz')"/>][<xsl:value-of select="str:padding(-1, 'x')"/>][<xsl:value-of select="str:padding(3, '')"/>]
    <xsl:text>&#10;align: </xsl:text>[<xsl:value-of select="str:align('ab', '-----')"/>][<xsl:value-of select="str:align('ab', '-----', 'right')"/>][<xsl:value-of select="str:align('ab', '-----', 'center')"/>][<xsl:value-of select="str:align('abcdef', '---')"/>]
    <xsl:text>&#10;replace: </xsl:text>[<xsl:value-of select="str:replace('a-b-c', '-', '+')"/>][<xsl:value-of select="str:replace('abc', '', 'X')"/>][<xsl:value-of select="str:replace('aabb', /r/s, 'Q')"/>][<xsl:value-of select="count(str:replace('', 'a', 'b'))"/>][<xsl:value-of select="str:replace('abcd', str:tokenize('ab bc'), str:tokenize('1'))"/>]
    <xsl:text>&#10;uri: </xsl:text><xsl:value-of select="str:encode-uri('a b/c?d=é@%', true())"/>|<xsl:value-of select="str:encode-uri('a b/c?d=é@%', false())"/>|<xsl:value-of select="str:decode-uri('a%20b%C3%A9%zz%')"/>
    <xsl:text>&#10;math: </xsl:text><xsl:value-of select="math:min($ns)"/>,<xsl:value-of select="math:max($ns)"/>,<xsl:value-of select="math:min(/r/nothing)"/>,<xsl:value-of select="math:max(/r/*)"/>,<xsl:value-of select="count(math:highest($ns))"/>,<xsl:value-of select="count(math:lowest($ns))"/>,<xsl:value-of select="count(math:lowest(/r/*))"/>
    <xsl:text>&#10;math2: </xsl:text><xsl:value-of select="math:abs(-2.5)"/>,<xsl:value-of select="math:sqrt(16)"/>,<xsl:value-of select="math:power(2, 10)"/>,<xsl:value-of select="math:constant('PI', 4)"/>,<xsl:value-of select="math:constant('E', 30)"/>,<xsl:value-of select="math:constant('PI', 0)"/>,<xsl:value-of select="math:constant('nope', 5)"/>,<xsl:value-of select="math:atan2(1, 0) * 2"/>,<xsl:value-of select="math:random() &lt; 1"/>
    <xsl:text>&#10;</xsl:text>
  </xsl:template>
</xsl:stylesheet>
