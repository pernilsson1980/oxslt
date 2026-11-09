<xsl:stylesheet version="1.0" xmlns:xsl="http://www.w3.org/1999/XSL/Transform"
  xmlns:exsl="http://exslt.org/common" xmlns:str="http://exslt.org/strings" xmlns:set="http://exslt.org/sets"
  extension-element-prefixes="exsl" exclude-result-prefixes="exsl str set">
  <xsl:output method="xml" indent="yes"/>
  <xsl:variable name="codes" select="document('lookup.xml')/codes"/>
  <xsl:variable name="rtf"><v>3</v><v>1</v><v>2</v><v>1</v></xsl:variable>
  <xsl:template match="/list">
    <r>
      <xsl:for-each select="x"><name k="{@k}"><xsl:value-of select="$codes/c[@k = current()/@k]"/></name></xsl:for-each>
      <sum><xsl:value-of select="sum(exsl:node-set($rtf)/v)"/></sum>
      <distinct><xsl:value-of select="count(set:distinct(exsl:node-set($rtf)/v))"/></distinct>
      <type><xsl:value-of select="exsl:object-type($rtf)"/></type>
      <xsl:for-each select="str:tokenize('a,b;c', ',;')"><tok><xsl:value-of select="."/></tok></xsl:for-each>
    </r>
  </xsl:template>
</xsl:stylesheet>
